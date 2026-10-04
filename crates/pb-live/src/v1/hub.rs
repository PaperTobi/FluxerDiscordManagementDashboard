//! Topic cells.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use pb_live_proto::{Topic, TopicDelta, TopicState};
use tokio::sync::broadcast;

/// What a subscriber receives after its snapshot.
#[derive(Debug, Clone)]
pub enum Update {
    Delta {
        rev: u64,
        delta: Arc<TopicDelta>,
    },
    /// The state was replaced as a whole: take a new snapshot.
    Reset,
}

struct Cell {
    rev: u64,
    state: TopicState,
    tx: broadcast::Sender<Update>,
}

/// A subscription: the state at `rev` and the updates after it.
#[derive(Debug)]
pub struct Subscription {
    pub rev: u64,
    pub state: TopicState,
    pub updates: broadcast::Receiver<Update>,
}

/// All cells.
#[derive(Clone, Default)]
pub struct Hub {
    cells: Arc<Mutex<HashMap<Topic, Cell>>>,
}

impl std::fmt::Debug for Hub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Hub").field("topics", &self.lock().len()).finish()
    }
}

/// Updates kept for a subscriber that is a little behind (one that falls further behind gets a fresh snapshot).
const BACKLOG: usize = 256;

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<Topic, Cell>> {
        self.cells.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Sets a topic's whole state (creating the cell); subscribers re-snapshot.
    pub fn set(&self, topic: Topic, state: TopicState) {
        let mut cells = self.lock();
        match cells.get_mut(&topic) {
            Some(c) => {
                c.rev += 1;
                c.state = state;
                let _ = c.tx.send(Update::Reset);
            }
            None => {
                let (tx, _) = broadcast::channel(BACKLOG);
                cells.insert(topic, Cell { rev: 1, state, tx });
            }
        }
    }

    /// Applies a change to a topic and broadcasts it. Returns `false` when the topic has no cell (nobody built it) or
    /// the delta does not fit it.
    pub fn publish(&self, topic: &Topic, delta: TopicDelta) -> bool {
        let mut cells = self.lock();
        let Some(c) = cells.get_mut(topic) else { return false };
        if c.state.apply(&delta).is_err() {
            tracing::error!(?topic, "a delta for another kind of topic was dropped");
            return false;
        }
        c.rev += 1;
        let _ = c.tx.send(Update::Delta {
            rev: c.rev,
            delta: Arc::new(delta),
        });
        true
    }

    /// Every topic with a cell.
    pub fn topics(&self) -> Vec<Topic> {
        self.lock().keys().cloned().collect()
    }

    /// Whether the topic has a cell.
    pub fn has(&self, topic: &Topic) -> bool {
        self.lock().contains_key(topic)
    }

    /// Whether anybody watches the topic (worth computing high-rate changes for).
    pub fn watched(&self, topic: &Topic) -> bool {
        self.lock().get(topic).is_some_and(|c| c.tx.receiver_count() > 0)
    }

    /// The current state, if the topic has a cell.
    pub fn state(&self, topic: &Topic) -> Option<TopicState> {
        self.lock().get(topic).map(|c| c.state.clone())
    }

    /// Subscribes: a consistent snapshot and the updates after it.
    pub fn subscribe(&self, topic: &Topic) -> Option<Subscription> {
        let cells = self.lock();
        let c = cells.get(topic)?;
        Some(Subscription {
            rev: c.rev,
            state: c.state.clone(),
            updates: c.tx.subscribe(),
        })
    }

    /// Removes the cells nobody watches, except those `keep` names (they are built again when someone watches).
    pub fn drop_unwatched(&self, keep: impl Fn(&Topic) -> bool) {
        self.lock().retain(|topic, c| keep(topic) || c.tx.receiver_count() > 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pb_domain::GuildId;
    use pb_live_proto::{SidebarDelta, SidebarState};

    #[tokio::test]
    async fn snapshot_then_deltas() {
        let hub = Hub::new();
        let t = Topic::Sidebar;
        assert!(hub.subscribe(&t).is_none());
        hub.set(t.clone(), TopicState::Sidebar(SidebarState::default()));
        hub.publish(
            &t,
            TopicDelta::Sidebar(SidebarDelta::CommunityGone { guild: GuildId(1) }),
        );
        let mut sub = hub.subscribe(&t).unwrap();
        assert_eq!(sub.rev, 2);
        let TopicState::Sidebar(s) = &sub.state else { panic!() };
        assert!(s.communities.is_empty());
        assert!(hub.watched(&t));
        hub.publish(
            &t,
            TopicDelta::Sidebar(SidebarDelta::CommunityGone { guild: GuildId(2) }),
        );
        let Update::Delta { rev, .. } = sub.updates.recv().await.unwrap() else {
            panic!()
        };
        assert_eq!(rev, 3);
        hub.set(t.clone(), TopicState::Sidebar(SidebarState::default()));
        assert!(matches!(sub.updates.recv().await.unwrap(), Update::Reset));
    }

    #[tokio::test]
    async fn a_slow_subscriber_lags_instead_of_growing_a_backlog() {
        let hub = Hub::new();
        let t = Topic::Sidebar;
        hub.set(t.clone(), TopicState::Sidebar(SidebarState::default()));
        let mut sub = hub.subscribe(&t).unwrap();
        for _ in 0..(BACKLOG + 10) {
            hub.publish(
                &t,
                TopicDelta::Sidebar(SidebarDelta::CommunityGone { guild: GuildId(1) }),
            );
        }
        assert!(matches!(
            sub.updates.recv().await,
            Err(broadcast::error::RecvError::Lagged(_))
        ));
    }
}
