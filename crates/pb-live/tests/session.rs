//! The live session against a scripted page.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use pb_domain::{GuildId, UserId};
use pb_live::{Access, CellSource, End, Hub, SessionCfg, SessionCtx, serve};
use pb_live_proto::*;
use tokio::sync::watch;

struct TestAccess {
    owner: bool,
    guilds: Vec<u64>,
    expired: AtomicBool,
    epoch: AtomicU64,
}

impl Access for TestAccess {
    fn owner(&self) -> bool {
        self.owner
    }
    fn guild(&self, g: GuildId) -> bool {
        self.guilds.contains(&g.0)
    }
    fn expired(&self) -> bool {
        self.expired.load(Ordering::SeqCst)
    }
    fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }
}

struct NoCells;
impl CellSource for NoCells {
    fn ensure(&self, _: &Topic) -> bool {
        false
    }
}

fn community(id: u64) -> SidebarCommunity {
    SidebarCommunity {
        id: GuildId(id),
        name: format!("c{id}"),
        icon: None,
        available: true,
        paused: false,
        people: vec![],
    }
}

fn hub() -> Hub {
    let hub = Hub::new();
    hub.set(
        Topic::Sidebar,
        TopicState::Sidebar(SidebarState {
            communities: vec![community(1), community(2)],
        }),
    );
    hub.set(Topic::Wall, TopicState::Wall(WallState::default()));
    hub
}

fn cfg() -> SessionCfg {
    SessionCfg {
        ping_every: Duration::from_secs(60),
        pong_timeout: Duration::from_secs(120),
        write_deadline: Duration::from_millis(300),
        access_check: Duration::from_millis(50),
    }
}

struct Page {
    tx: mpsc::UnboundedSender<ClientMsg>,
    rx: mpsc::Receiver<ServerMsg>,
    end: tokio::task::JoinHandle<End>,
    stop: watch::Sender<bool>,
}

fn open(hub: &Hub, access: Arc<TestAccess>, cfg: SessionCfg, capacity: usize) -> Page {
    let (tx, incoming) = mpsc::unbounded();
    let (outgoing, rx) = mpsc::channel(capacity);
    let (stop, stop_rx) = watch::channel(false);
    let ctx = SessionCtx {
        hub: hub.clone(),
        access,
        source: Arc::new(NoCells),
        cfg,
        version: "1.0".into(),
        shutdown: stop_rx,
    };
    let end = tokio::spawn(serve(ctx, incoming, outgoing));
    Page { tx, rx, end, stop }
}

fn owner() -> Arc<TestAccess> {
    Arc::new(TestAccess {
        owner: true,
        guilds: vec![],
        expired: AtomicBool::new(false),
        epoch: AtomicU64::new(0),
    })
}

async fn next(p: &mut Page) -> ServerMsg {
    tokio::time::timeout(Duration::from_secs(2), p.rx.next())
        .await
        .expect("a frame")
        .expect("open")
}

fn hello(view: u64, visible: bool, topics: Vec<Topic>) -> ClientMsg {
    ClientMsg::Hello {
        proto: PROTO,
        view,
        visible,
        topics,
    }
}

/// A cheap sidebar change (a community this test never shows going away).
fn sidebar_change() -> TopicDelta {
    TopicDelta::Sidebar(SidebarDelta::CommunityGone { guild: GuildId(99) })
}

#[tokio::test]
async fn snapshot_then_deltas_tagged_with_the_view() {
    let hub = hub();
    let mut p = open(&hub, owner(), cfg(), 64);
    p.tx.unbounded_send(hello(7, true, vec![Topic::Sidebar])).unwrap();
    assert!(matches!(next(&mut p).await, ServerMsg::Welcome { proto: PROTO, .. }));
    let ServerMsg::Snapshot { view, rev, state, .. } = next(&mut p).await else {
        panic!()
    };
    assert_eq!((view, rev), (7, 1));
    assert!(matches!(*state, TopicState::Sidebar(_)));
    hub.publish(&Topic::Sidebar, sidebar_change());
    let ServerMsg::Delta { view, rev, .. } = next(&mut p).await else {
        panic!()
    };
    assert_eq!((view, rev), (7, 2));
    // A page that applies them stays in step.
    let mut t = TopicTracker::new(true);
    t.want(Topic::Sidebar);
    let ClientMsg::Hello { view: v, .. } = t.hello() else {
        panic!()
    };
    assert_eq!(v, 2);
    p.stop.send_replace(true);
    assert!(matches!(next(&mut p).await, ServerMsg::Shutdown { .. }));
    assert_eq!(p.end.await.unwrap(), End::Shutdown);
}

#[tokio::test]
async fn a_hidden_tab_keeps_only_the_sidebar_and_comes_back_with_snapshots() {
    let hub = hub();
    let mut p = open(&hub, owner(), cfg(), 64);
    p.tx.unbounded_send(hello(1, true, vec![Topic::Sidebar, Topic::Wall]))
        .unwrap();
    for _ in 0..3 {
        next(&mut p).await;
    }
    p.tx.unbounded_send(ClientMsg::Visibility {
        view: 1,
        visible: false,
    })
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let tile = Tile {
        guild: GuildId(1),
        community: "c1".into(),
        who: Who {
            user: UserId(5),
            name: "p".into(),
            avatar: None,
        },
        channel: ChannelRef {
            id: pb_domain::ChannelId(9),
            name: "v".into(),
        },
        levels: Levels::default(),
        latest: None,
        lag_ms: None,
    };
    hub.publish(&Topic::Wall, TopicDelta::Wall(WallDelta::Tile { tile: Box::new(tile) }));
    hub.publish(&Topic::Sidebar, sidebar_change());
    let ServerMsg::Delta { topic, .. } = next(&mut p).await else {
        panic!()
    };
    assert_eq!(topic, Topic::Sidebar, "nothing about the wall while hidden");
    p.tx.unbounded_send(ClientMsg::Visibility { view: 2, visible: true })
        .unwrap();
    let mut snaps = Vec::new();
    for _ in 0..2 {
        let ServerMsg::Snapshot { view, topic, state, .. } = next(&mut p).await else {
            panic!()
        };
        assert_eq!(view, 2);
        if let TopicState::Wall(w) = *state {
            assert_eq!(w.tiles.len(), 1, "the wall comes back as it is now");
        }
        snaps.push(topic);
    }
    snaps.sort();
    assert_eq!(snaps, vec![Topic::Sidebar, Topic::Wall]);
}

#[tokio::test]
async fn a_frozen_page_is_dropped_at_the_write_deadline() {
    let hub = hub();
    let p = open(&hub, owner(), cfg(), 1);
    p.tx.unbounded_send(hello(1, true, vec![Topic::Sidebar])).unwrap();
    for _ in 0..20 {
        hub.publish(&Topic::Sidebar, sidebar_change());
    }
    let end = tokio::time::timeout(Duration::from_secs(3), p.end)
        .await
        .expect("ends")
        .unwrap();
    assert_eq!(end, End::WriteTimeout);
}

#[tokio::test]
async fn a_page_that_fell_behind_gets_a_snapshot_not_a_backlog() {
    let hub = hub();
    let mut p = open(
        &hub,
        owner(),
        SessionCfg {
            write_deadline: Duration::from_secs(5),
            ..cfg()
        },
        1,
    );
    p.tx.unbounded_send(hello(1, true, vec![Topic::Sidebar])).unwrap();
    // The page reads nothing while 600 changes happen (more than a cell keeps for a slow reader).
    tokio::time::sleep(Duration::from_millis(50)).await;
    for _ in 0..600 {
        hub.publish(&Topic::Sidebar, sidebar_change());
    }
    let mut frames = 0;
    let mut snapshots = 0;
    let mut last_rev = 0;
    while let Ok(Some(f)) = tokio::time::timeout(Duration::from_millis(300), p.rx.next()).await {
        frames += 1;
        match f {
            ServerMsg::Snapshot { rev, .. } => {
                snapshots += 1;
                last_rev = rev;
            }
            ServerMsg::Delta { rev, .. } => last_rev = rev,
            _ => {}
        }
    }
    assert!(snapshots >= 2, "a fresh snapshot after the lag");
    assert!(frames < 400, "not a replay of every change: {frames} frames");
    assert_eq!(last_rev, 601, "ends at the newest state");
}

#[tokio::test]
async fn access_filters_denies_reacts_to_changes_and_expiry() {
    let hub = hub();
    let access = Arc::new(TestAccess {
        owner: false,
        guilds: vec![1],
        expired: AtomicBool::new(false),
        epoch: AtomicU64::new(0),
    });
    let mut p = open(&hub, access.clone(), cfg(), 64);
    p.tx.unbounded_send(hello(
        1,
        true,
        vec![Topic::Sidebar, Topic::System, Topic::Guild { guild: GuildId(2) }],
    ))
    .unwrap();
    next(&mut p).await;
    let mut got = Vec::new();
    for _ in 0..3 {
        got.push(next(&mut p).await);
    }
    let sidebar = got.iter().find_map(|f| match f {
        ServerMsg::Snapshot { state, .. } => match &**state {
            TopicState::Sidebar(s) => Some(s.communities.iter().map(|c| c.id.0).collect::<Vec<_>>()),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(sidebar, Some(vec![1]), "only the communities this login may see");
    assert!(got.iter().any(|f| matches!(
        f,
        ServerMsg::Denied {
            topic: Topic::System,
            reason: DenyReason::NotAllowed,
            ..
        }
    )));
    assert!(got.iter().any(|f| matches!(
        f,
        ServerMsg::Denied {
            topic: Topic::Guild { .. },
            reason: DenyReason::NotAllowed,
            ..
        }
    )));
    // Deltas about another community are not sent, and the page's revisions stay continuous.
    let rev0 = got
        .iter()
        .find_map(|f| match f {
            ServerMsg::Snapshot {
                topic: Topic::Sidebar,
                rev,
                ..
            } => Some(*rev),
            _ => None,
        })
        .unwrap();
    for g in [3, 1] {
        hub.publish(
            &Topic::Sidebar,
            TopicDelta::Sidebar(SidebarDelta::Community {
                community: community(g),
            }),
        );
    }
    let ServerMsg::Delta { rev, delta, .. } = next(&mut p).await else {
        panic!("only the delta this login may see")
    };
    assert_eq!(rev, rev0 + 1);
    assert!(matches!(*delta, TopicDelta::Sidebar(SidebarDelta::Community { community }) if community.id.0 == 1));
    access.epoch.store(1, Ordering::SeqCst);
    assert!(matches!(next(&mut p).await, ServerMsg::AccessChanged));
    access.expired.store(true, Ordering::SeqCst);
    loop {
        if matches!(next(&mut p).await, ServerMsg::AuthExpired) {
            break;
        }
    }
    assert_eq!(p.end.await.unwrap(), End::AuthExpired);
}

#[tokio::test]
async fn pings_detect_a_dead_page_and_versions_must_match() {
    let hub = hub();
    let mut p = open(
        &hub,
        owner(),
        SessionCfg {
            ping_every: Duration::from_millis(40),
            pong_timeout: Duration::from_millis(150),
            ..cfg()
        },
        64,
    );
    p.tx.unbounded_send(hello(1, true, vec![])).unwrap();
    next(&mut p).await;
    let ServerMsg::Ping { nonce, .. } = next(&mut p).await else {
        panic!()
    };
    p.tx.unbounded_send(ClientMsg::Pong { nonce }).unwrap();
    let end = tokio::time::timeout(Duration::from_secs(2), async { while p.rx.next().await.is_some() {} }).await;
    assert!(end.is_ok());
    assert_eq!(p.end.await.unwrap(), End::NoPong);
    let mut old = open(&hub, owner(), cfg(), 64);
    old.tx
        .unbounded_send(ClientMsg::Hello {
            proto: 0,
            view: 1,
            visible: true,
            topics: vec![],
        })
        .unwrap();
    assert!(matches!(next(&mut old).await, ServerMsg::Reload { .. }));
    assert_eq!(old.end.await.unwrap(), End::Version);
}
