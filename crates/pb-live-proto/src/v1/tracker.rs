//! The page's bookkeeping per topic: which topics it wants, the state and `rev` of each, and the `view` of its current
//! view.

use std::collections::BTreeMap;

use super::state::TopicState;
use super::wire::{ClientMsg, DenyReason, PROTO, ReloadReason, ServerMs, ServerMsg, Topic};

#[derive(Debug, Clone)]
struct Entry {
    rev: u64,
    state: Option<TopicState>,
    /// A snapshot for the current `view` is due (deltas are ignored until it arrives).
    waiting: bool,
    denied: Option<DenyReason>,
}

/// What a received frame means for the page.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// A topic's state changed.
    Changed(Topic),
    /// Nothing to do (an old `view`, a topic no longer wanted, a duplicate).
    Ignored,
    /// Send this message.
    Send(ClientMsg),
    Welcome {
        server_ms: ServerMs,
        version: String,
    },
    /// Answer the ping (and update the clock).
    Ping {
        pong: ClientMsg,
        server_ms: ServerMs,
        rtt_ms: Option<u32>,
    },
    Denied(Topic, DenyReason),
    /// Ask again for these denied topics.
    Retry(Vec<ClientMsg>),
    AuthExpired,
    Shutdown {
        restarting: bool,
    },
    Reload(ReloadReason),
    /// A frame that cannot be applied (a delta for the wrong kind of topic): a bug; the topic is resynced.
    Broken(Topic, ClientMsg),
}

/// The topics a page watches.
#[derive(Debug, Clone, Default)]
pub struct TopicTracker {
    view: u64,
    visible: bool,
    topics: BTreeMap<Topic, Entry>,
}

impl TopicTracker {
    pub fn new(visible: bool) -> TopicTracker {
        TopicTracker {
            view: 1,
            visible,
            topics: BTreeMap::new(),
        }
    }

    pub fn view(&self) -> u64 {
        self.view
    }

    pub fn visible(&self) -> bool {
        self.visible
    }

    pub fn state(&self, topic: &Topic) -> Option<&TopicState> {
        self.topics.get(topic).and_then(|e| e.state.as_ref())
    }

    /// Up to date: a snapshot for the current view arrived and no gap since.
    pub fn fresh(&self, topic: &Topic) -> bool {
        self.topics.get(topic).is_some_and(|e| !e.waiting && e.state.is_some())
    }

    pub fn denied(&self, topic: &Topic) -> Option<DenyReason> {
        self.topics.get(topic).and_then(|e| e.denied)
    }

    pub fn topics(&self) -> impl Iterator<Item = &Topic> {
        self.topics.keys()
    }

    /// The first message on a new connection. Old states stay on screen until their snapshots arrive.
    pub fn hello(&mut self) -> ClientMsg {
        self.view += 1;
        for e in self.topics.values_mut() {
            e.waiting = true;
            e.denied = None;
        }
        ClientMsg::Hello {
            proto: PROTO,
            view: self.view,
            visible: self.visible,
            topics: self.topics.keys().cloned().collect(),
        }
    }

    /// Start watching a topic (`None` if already watched).
    pub fn want(&mut self, topic: Topic) -> Option<ClientMsg> {
        if self.topics.contains_key(&topic) {
            return None;
        }
        self.topics.insert(
            topic.clone(),
            Entry {
                rev: 0,
                state: None,
                waiting: true,
                denied: None,
            },
        );
        Some(ClientMsg::Sub { view: self.view, topic })
    }

    /// Stop watching a topic (`None` if not watched).
    pub fn unwant(&mut self, topic: &Topic) -> Option<ClientMsg> {
        self.topics
            .remove(topic)
            .map(|_| ClientMsg::Unsub { topic: topic.clone() })
    }

    /// The tab was hidden or shown. Showing starts a new view: everything waits for fresh snapshots.
    pub fn set_visible(&mut self, visible: bool) -> Option<ClientMsg> {
        if visible == self.visible {
            return None;
        }
        self.visible = visible;
        if visible {
            self.view += 1;
            for e in self.topics.values_mut() {
                e.waiting = true;
            }
        }
        Some(ClientMsg::Visibility {
            view: self.view,
            visible,
        })
    }

    pub fn receive(&mut self, msg: ServerMsg) -> Outcome {
        if msg.view().is_some_and(|g| g != self.view) {
            return Outcome::Ignored;
        }
        match msg {
            ServerMsg::Welcome { server_ms, version, .. } => Outcome::Welcome { server_ms, version },
            ServerMsg::Snapshot { topic, rev, state, .. } => {
                let Some(e) = self.topics.get_mut(&topic) else {
                    return Outcome::Ignored;
                };
                if !state.fits(&topic) {
                    e.waiting = true;
                    return Outcome::Broken(topic.clone(), ClientMsg::Resync { view: self.view, topic });
                }
                e.rev = rev;
                e.state = Some(*state);
                e.waiting = false;
                e.denied = None;
                Outcome::Changed(topic)
            }
            ServerMsg::Delta { topic, rev, delta, .. } => {
                let view = self.view;
                let Some(e) = self.topics.get_mut(&topic) else {
                    return Outcome::Ignored;
                };
                if e.waiting || rev <= e.rev {
                    return Outcome::Ignored;
                }
                let Some(state) = e.state.as_mut().filter(|_| rev == e.rev + 1) else {
                    e.waiting = true;
                    return Outcome::Send(ClientMsg::Resync { view, topic });
                };
                if state.apply(&delta).is_err() {
                    e.waiting = true;
                    return Outcome::Broken(topic.clone(), ClientMsg::Resync { view, topic });
                }
                e.rev = rev;
                Outcome::Changed(topic)
            }
            ServerMsg::Denied { topic, reason, .. } => {
                let Some(e) = self.topics.get_mut(&topic) else {
                    return Outcome::Ignored;
                };
                e.denied = Some(reason);
                e.state = None;
                e.waiting = false;
                Outcome::Denied(topic, reason)
            }
            ServerMsg::AccessChanged => {
                let view = self.view;
                let again: Vec<ClientMsg> = self
                    .topics
                    .iter_mut()
                    .filter(|(_, e)| e.denied.is_some())
                    .map(|(t, e)| {
                        e.denied = None;
                        e.waiting = true;
                        ClientMsg::Sub { view, topic: t.clone() }
                    })
                    .collect();
                if again.is_empty() {
                    Outcome::Ignored
                } else {
                    Outcome::Retry(again)
                }
            }
            ServerMsg::AuthExpired => Outcome::AuthExpired,
            ServerMsg::Ping {
                nonce,
                server_ms,
                rtt_ms,
            } => Outcome::Ping {
                pong: ClientMsg::Pong { nonce },
                server_ms,
                rtt_ms,
            },
            ServerMsg::Shutdown { restarting } => Outcome::Shutdown { restarting },
            ServerMsg::Reload { reason } => Outcome::Reload(reason),
        }
    }
}
