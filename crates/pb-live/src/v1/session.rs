//! One live connection, independent of the transport (the web server pumps frames in and out).
//!
//! The client says which topics it shows; the session sends a snapshot of each, then its deltas, every frame tagged
//! with the client's current `view`. A hidden tab keeps only the sidebar. A subscriber that falls behind gets a fresh
//! snapshot, never a backlog; a client that does not read within the write deadline is dropped at once; pings detect a
//! dead connection. Sidebar and wall are filtered to the communities this login may see.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::{Sink, SinkExt, Stream, StreamExt};
use pb_domain::GuildId;
use pb_live_proto::{
    ClientMsg, DenyReason, PROTO, ReloadReason, ServerMs, ServerMsg, SidebarDelta, SidebarState, Topic, TopicDelta,
    TopicState, WallDelta, WallState,
};
use tokio_stream::StreamMap;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

use super::hub::{Hub, Update};

/// What a login may see.
pub trait Access: Send + Sync {
    /// The bot owner sees everything.
    fn owner(&self) -> bool;
    /// May see this community (its page, people and entries in the sidebar and wall).
    fn guild(&self, g: GuildId) -> bool;
    /// The login ended (logged out, expired).
    fn expired(&self) -> bool;
    /// Changes whenever what the login may see changes.
    fn epoch(&self) -> u64;
}

/// Builds cells on demand (a community or person nobody watched yet).
pub trait CellSource: Send + Sync {
    /// Makes sure the topic has a cell; `false` = no such community or person.
    fn ensure(&self, topic: &Topic) -> bool;
}

/// Timings.
#[derive(Debug, Clone)]
pub struct SessionCfg {
    pub ping_every: Duration,
    pub pong_timeout: Duration,
    /// A frame the client does not take within this time ends the connection (a frozen tab).
    pub write_deadline: Duration,
    /// How often the login's access is checked.
    pub access_check: Duration,
}

impl Default for SessionCfg {
    fn default() -> Self {
        SessionCfg {
            ping_every: Duration::from_secs(15),
            pong_timeout: Duration::from_secs(45),
            write_deadline: Duration::from_secs(10),
            access_check: Duration::from_secs(5),
        }
    }
}

/// Why a session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum End {
    ClientClosed,
    /// The client did not take a frame within the write deadline.
    WriteTimeout,
    /// No pong within the timeout.
    NoPong,
    AuthExpired,
    /// Another protocol version (the page was told to reload).
    Version,
    Shutdown,
}

fn now_ms() -> ServerMs {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
}

fn allowed(access: &dyn Access, topic: &Topic) -> Result<(), DenyReason> {
    let ok = match topic {
        Topic::System => access.owner(),
        Topic::Guild { guild } | Topic::Person { guild, .. } => access.owner() || access.guild(*guild),
        Topic::Sidebar | Topic::Wall => true,
    };
    if ok { Ok(()) } else { Err(DenyReason::NotAllowed) }
}

/// The part of a state this login may see.
fn filter_state(access: &dyn Access, state: TopicState) -> TopicState {
    if access.owner() {
        return state;
    }
    match state {
        TopicState::Sidebar(s) => TopicState::Sidebar(SidebarState {
            communities: s.communities.into_iter().filter(|c| access.guild(c.id)).collect(),
        }),
        TopicState::Wall(w) => TopicState::Wall(WallState {
            tiles: w.tiles.into_iter().filter(|t| access.guild(t.guild)).collect(),
            violations: w.violations.into_iter().filter(|v| access.guild(v.guild)).collect(),
        }),
        other => other,
    }
}

/// Whether this login may see a delta (sidebar and wall deltas about other communities are not sent).
fn delta_visible(access: &dyn Access, d: &TopicDelta) -> bool {
    if access.owner() {
        return true;
    }
    match d {
        TopicDelta::Sidebar(SidebarDelta::Community { community }) => access.guild(community.id),
        TopicDelta::Sidebar(SidebarDelta::CommunityGone { guild } | SidebarDelta::Dot { guild, .. }) => {
            access.guild(*guild)
        }
        TopicDelta::Wall(
            WallDelta::TileGone { guild, .. } | WallDelta::Levels { guild, .. } | WallDelta::Sentence { guild, .. },
        ) => access.guild(*guild),
        TopicDelta::Wall(WallDelta::Tile { tile }) => access.guild(tile.guild),
        TopicDelta::Wall(WallDelta::Violation { violation }) => access.guild(violation.guild),
        // Topics a login only gets when it may see all of them.
        TopicDelta::Guild(_) | TopicDelta::Person(_) | TopicDelta::System(_) => true,
    }
}

/// A watched topic's revision in the cell and as the client counts it.
#[derive(Debug, Clone, Copy)]
struct Revs {
    cell: u64,
    client: u64,
}

/// What a session runs on.
#[derive(Clone)]
pub struct SessionCtx {
    pub hub: Hub,
    pub access: Arc<dyn Access>,
    pub source: Arc<dyn CellSource>,
    pub cfg: SessionCfg,
    /// The bot's version (sent in `Welcome`).
    pub version: String,
    /// Becomes `true` when the bot stops.
    pub shutdown: tokio::sync::watch::Receiver<bool>,
}

impl std::fmt::Debug for SessionCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionCtx")
            .field("cfg", &self.cfg)
            .finish_non_exhaustive()
    }
}

/// Runs a session until the connection ends. `incoming` yields the client's messages (it ends when the client goes);
/// `outgoing` takes the bot's.
#[allow(clippy::too_many_lines)]
pub async fn serve<I, O>(ctx: SessionCtx, mut incoming: I, mut outgoing: O) -> End
where
    I: Stream<Item = ClientMsg> + Unpin,
    O: Sink<ServerMsg> + Unpin,
{
    let SessionCtx {
        hub,
        access,
        source,
        cfg,
        version,
        mut shutdown,
    } = ctx;
    let mut view = 0u64;
    let mut visible = true;
    let mut wanted: BTreeSet<Topic> = BTreeSet::new();
    // Each watched topic: the cell's revision and the client's (deltas this login may not see are skipped, so the
    // client counts only what it gets), and its updates read straight from the cell's broadcast (a slow client lets
    // them pile up there, bounded, until a lag turns into a fresh snapshot; nothing is buffered here).
    let mut revs: BTreeMap<Topic, Revs> = BTreeMap::new();
    let mut streams: StreamMap<Topic, BroadcastStream<Update>> = StreamMap::new();
    let mut last_pong = Instant::now();
    let mut nonce = 0u64;
    let mut ping_sent: Option<(u64, Instant)> = None;
    let mut rtt: Option<u32> = None;
    let mut epoch = access.epoch();
    // The first ping one period after the start (an interval's first tick would fire at once).
    let mut ping = tokio::time::interval_at(tokio::time::Instant::now() + cfg.ping_every, cfg.ping_every);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut check = tokio::time::interval(cfg.access_check);
    check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    // Sends a frame within the write deadline, or ends the session (the label is passed in: labels are hygienic).
    macro_rules! send {
        ($l:lifetime, $msg:expr) => {
            match tokio::time::timeout(cfg.write_deadline, outgoing.send($msg)).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => break $l End::ClientClosed,
                Err(_) => break $l End::WriteTimeout,
            }
        };
    }

    // Subscribes (or re-snapshots) a topic: the snapshot frame to send, or why not.
    let subscribe = |topic: &Topic,
                     revs: &mut BTreeMap<Topic, Revs>,
                     streams: &mut StreamMap<Topic, BroadcastStream<Update>>,
                     view: u64|
     -> Result<ServerMsg, ServerMsg> {
        if let Err(reason) = allowed(&*access, topic) {
            return Err(ServerMsg::Denied {
                view,
                topic: topic.clone(),
                reason,
            });
        }
        if !hub.has(topic) && !source.ensure(topic) {
            return Err(ServerMsg::Denied {
                view,
                topic: topic.clone(),
                reason: DenyReason::NotFound,
            });
        }
        let Some(sub) = hub.subscribe(topic) else {
            return Err(ServerMsg::Denied {
                view,
                topic: topic.clone(),
                reason: DenyReason::NotFound,
            });
        };
        streams.insert(topic.clone(), BroadcastStream::new(sub.updates));
        // The cell's revision only grows and the client's never passes it, so a snapshot can start the client there.
        revs.insert(
            topic.clone(),
            Revs {
                cell: sub.rev,
                client: sub.rev,
            },
        );
        Ok(ServerMsg::Snapshot {
            view,
            topic: topic.clone(),
            rev: sub.rev,
            state: Box::new(filter_state(&*access, sub.state)),
        })
    };
    let drop_topic =
        |topic: &Topic, revs: &mut BTreeMap<Topic, Revs>, streams: &mut StreamMap<Topic, BroadcastStream<Update>>| {
            revs.remove(topic);
            streams.remove(topic);
        };
    let active = |topic: &Topic, visible: bool| visible || *topic == Topic::Sidebar;

    'session: loop {
        tokio::select! {
            msg = incoming.next() => {
                let Some(msg) = msg else { break End::ClientClosed };
                match msg {
                    ClientMsg::Hello { proto, view: v, visible: vis, topics } => {
                        if proto != PROTO {
                            send!('session, ServerMsg::Reload { reason: ReloadReason::NewVersion });
                            break End::Version;
                        }
                        view = v;
                        visible = vis;
                        send!('session, ServerMsg::Welcome { proto: PROTO, server_ms: now_ms(), version: version.clone() });
                        // A new start: only what this hello asks for.
                        revs.clear();
                        streams = StreamMap::new();
                        // Refused topics are answered and not kept (the page asks again after an access change).
                        wanted = BTreeSet::new();
                        for t in topics {
                            let ok = allowed(&*access, &t).is_ok();
                            if ok {
                                wanted.insert(t.clone());
                            }
                            if !ok || active(&t, visible) {
                                let frame = subscribe(&t, &mut revs, &mut streams, view).unwrap_or_else(|denied| denied);
                                send!('session, frame);
                            }
                        }
                    }
                    ClientMsg::Sub { view: v, topic } => {
                        view = view.max(v);
                        let ok = allowed(&*access, &topic).is_ok();
                        if ok {
                            wanted.insert(topic.clone());
                        }
                        if !ok || active(&topic, visible) {
                            let frame = subscribe(&topic, &mut revs, &mut streams, view).unwrap_or_else(|denied| denied);
                            send!('session, frame);
                        }
                    }
                    ClientMsg::Unsub { topic } => {
                        wanted.remove(&topic);
                        drop_topic(&topic, &mut revs, &mut streams);
                    }
                    ClientMsg::Visibility { view: v, visible: vis } => {
                        view = view.max(v);
                        visible = vis;
                        if visible {
                            // A new view: the page waits for a snapshot of everything it shows.
                            let list: Vec<Topic> = wanted.iter().cloned().collect();
                            for t in list {
                                let frame = subscribe(&t, &mut revs, &mut streams, view).unwrap_or_else(|denied| denied);
                                send!('session, frame);
                            }
                        } else {
                            // A hidden tab keeps only the sidebar (its dots); everything else re-snapshots on return.
                            let hidden: Vec<Topic> = revs.keys().filter(|t| **t != Topic::Sidebar).cloned().collect();
                            for t in hidden {
                                drop_topic(&t, &mut revs, &mut streams);
                            }
                        }
                    }
                    ClientMsg::Resync { view: v, topic } => {
                        view = view.max(v);
                        if wanted.contains(&topic) && active(&topic, visible) {
                            let frame = subscribe(&topic, &mut revs, &mut streams, view).unwrap_or_else(|denied| denied);
                            send!('session, frame);
                        }
                    }
                    ClientMsg::Pong { nonce: n } => {
                        last_pong = Instant::now();
                        if let Some((sent_n, at)) = ping_sent
                            && sent_n == n {
                                rtt = u32::try_from(at.elapsed().as_millis()).ok();
                            }
                    }
                }
            }
            Some((topic, item)) = streams.next(), if !streams.is_empty() => {
                let Some(r) = revs.get(&topic).copied() else { continue };
                match item {
                    Ok(Update::Delta { rev, delta }) if rev == r.cell + 1 => {
                        let visible_delta = delta_visible(&*access, &delta);
                        let client = r.client + u64::from(visible_delta);
                        revs.insert(topic.clone(), Revs { cell: rev, client });
                        if visible_delta {
                            send!('session, ServerMsg::Delta { view, topic, rev: client, delta: Box::new((*delta).clone()) });
                        }
                    }
                    Ok(Update::Delta { rev, .. }) if rev <= r.cell => {}
                    // A gap, a reset, or a lag: a fresh snapshot instead of a backlog.
                    Ok(_) | Err(BroadcastStreamRecvError::Lagged(_)) => {
                        let frame = subscribe(&topic, &mut revs, &mut streams, view).unwrap_or_else(|denied| denied);
                        send!('session, frame);
                    }
                }
            }
            _ = ping.tick() => {
                if last_pong.elapsed() > cfg.pong_timeout {
                    break End::NoPong;
                }
                nonce += 1;
                ping_sent = Some((nonce, Instant::now()));
                send!('session, ServerMsg::Ping { nonce, server_ms: now_ms(), rtt_ms: rtt });
            }
            _ = check.tick() => {
                if access.expired() {
                    let _ = tokio::time::timeout(cfg.write_deadline, outgoing.send(ServerMsg::AuthExpired)).await;
                    break End::AuthExpired;
                }
                let e = access.epoch();
                if e != epoch {
                    epoch = e;
                    send!('session, ServerMsg::AccessChanged);
                    // What it may see changed: everything shown is sent again (filtered), refusals included.
                    let list: Vec<Topic> = wanted.iter().filter(|t| active(t, visible)).cloned().collect();
                    for t in list {
                        let frame = subscribe(&t, &mut revs, &mut streams, view).unwrap_or_else(|denied| denied);
                        if matches!(frame, ServerMsg::Denied { .. }) {
                            drop_topic(&t, &mut revs, &mut streams);
                        }
                        send!('session, frame);
                    }
                }
            }
            _ = shutdown.changed() => {
                let _ = tokio::time::timeout(cfg.write_deadline, outgoing.send(ServerMsg::Shutdown { restarting: false })).await;
                break End::Shutdown;
            }
        }
    }
}
