#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use pb_domain::{Audience, ChannelId, ClfLang, GuildId, Label, SentenceId, UserId};
use pb_live_proto::*;

fn who(u: u64) -> Who {
    Who {
        user: UserId(u),
        name: format!("p{u}"),
        avatar: None,
    }
}

fn person(g: u64, u: u64) -> PersonState {
    PersonState {
        guild: GuildId(g),
        who: who(u),
        presence: Presence::default(),
        tracking: Tracking::default(),
        counts: Counts::default(),
        summary: PersonSummary {
            observe_only: false,
            audience: Audience::Tracked,
            strikes: 1,
            thresholds: vec![(Label::Profanity, 0.5)],
        },
        levels: Levels::default(),
        sentences: Vec::new(),
        activity: Default::default(),
    }
}

fn card(no: u32, stamps: Stamps) -> SentenceCard {
    SentenceCard {
        id: SentenceId::new(),
        no,
        stamps,
        dur_ms: Some(1200),
        level_db: Some(-23.5),
        cut: Some(CutWhy::Pause),
        dropped: None,
        error: None,
        verdict: Some(VerdictView {
            scores: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.9, 0.1],
            thresholds: vec![(Label::Profanity, 0.5)],
            flagged: vec![Label::Profanity],
            language: ClfLang::De,
            infer_ms: 140,
            cut_to_verdict_ms: 220,
        }),
        decision: Some(DecisionView::Warn { step: 1, count: 1 }),
    }
}

fn finished(t: i64) -> Stamps {
    Stamps {
        opened: t,
        cut: Some(t + 1000),
        queued: Some(t + 1001),
        scoring: Some(t + 1002),
        scored: Some(t + 1100),
        decided: Some(t + 1101),
        ..Stamps::default()
    }
}

#[test]
fn messages_round_trip_as_json() {
    let topic = Topic::Person {
        guild: GuildId(1),
        user: UserId(2),
    };
    let mut st = person(1, 2);
    st.sentences.push(card(1, finished(0)));
    let msgs = vec![
        ServerMsg::Welcome {
            proto: PROTO,
            server_ms: 5,
            version: "0.1.0".into(),
        },
        ServerMsg::Snapshot {
            view: 3,
            topic: topic.clone(),
            rev: 9,
            state: Box::new(TopicState::Person(Box::new(st))),
        },
        ServerMsg::Delta {
            view: 3,
            topic: topic.clone(),
            rev: 10,
            delta: Box::new(TopicDelta::Person(PersonDelta::Levels {
                run: LevelRun {
                    end_ms: 64,
                    frame_ms: 32,
                    frames: vec![LevelFrame::new(0.7, -20.0); 2],
                },
            })),
        },
        ServerMsg::Denied {
            view: 3,
            topic: Topic::System,
            reason: DenyReason::NotAllowed,
        },
        ServerMsg::AccessChanged,
        ServerMsg::AuthExpired,
        ServerMsg::Ping {
            nonce: 1,
            server_ms: 2,
            rtt_ms: Some(30),
        },
        ServerMsg::Shutdown { restarting: true },
        ServerMsg::Reload {
            reason: ReloadReason::NewVersion,
        },
    ];
    for m in msgs {
        let json = serde_json::to_string(&m).unwrap_or_default();
        let back: ServerMsg = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json}"));
        assert_eq!(back, m, "{json}");
    }
    let c = ClientMsg::Hello {
        proto: PROTO,
        view: 1,
        visible: true,
        topics: vec![Topic::Sidebar, topic],
    };
    let json = serde_json::to_string(&c).unwrap_or_default();
    assert!(
        json.contains(r#""t":"hello""#) && json.contains(r#""kind":"person","guild":"1","user":"2""#),
        "{json}"
    );
    assert_eq!(serde_json::from_str::<ClientMsg>(&json).ok(), Some(c));
}

#[test]
fn conveyor_dwells_and_never_runs_ahead() {
    let s = finished(0);
    assert_eq!(display_stage(&s, 500).station, Station::Recording);
    assert_eq!(display_stage(&s, 1000).station, Station::Cut);
    // Queued arrives 300 ms after Cut, Model 300 ms later, Verdict 400 ms later, Decision 700 ms later.
    assert_eq!(display_stage(&s, 1299).station, Station::Cut);
    assert_eq!(display_stage(&s, 1300).station, Station::Queued);
    assert_eq!(display_stage(&s, 1600).station, Station::Model);
    assert_eq!(display_stage(&s, 2000).station, Station::Verdict);
    let d = display_stage(&s, 2700);
    assert_eq!((d.station, d.since), (Station::Decision, 2700));
    // A sentence still being scored is never shown past the model.
    let open = Stamps {
        scored: None,
        decided: None,
        ..s
    };
    assert_eq!(display_stage(&open, 60_000).station, Station::Model);
    // A tab that comes back an hour later shows the end state at once.
    assert_eq!(display_stage(&s, 3_600_000).station, Station::Decision);
}

#[test]
fn conveyor_dropped_failed_and_playing() {
    let dropped = Stamps {
        opened: 0,
        dropped: Some(400),
        ..Stamps::default()
    };
    let d = display_stage(&dropped, 500);
    assert!(d.dropped && !d.gone && d.station == Station::Cut);
    assert!(display_stage(&dropped, 400 + DROPPED_VISIBLE_MS).gone);
    let failed = Stamps {
        opened: 0,
        cut: Some(10),
        queued: Some(11),
        scoring: Some(12),
        failed: Some(20),
        ..Stamps::default()
    };
    let f = display_stage(&failed, 10_000);
    assert!(f.failed && f.station == Station::Decision);
    let mut p = finished(0);
    p.play_start = Some(3000);
    assert!(!display_stage(&p, 2999).playing);
    assert!(display_stage(&p, 3000).playing);
    p.play_end = Some(4000);
    assert!(!display_stage(&p, 4000).playing);
}

#[test]
fn tracker_snapshots_deltas_gaps_and_views() {
    let topic = Topic::Person {
        guild: GuildId(1),
        user: UserId(2),
    };
    let mut t = TopicTracker::new(true);
    assert!(matches!(t.want(topic.clone()), Some(ClientMsg::Sub { .. })));
    assert!(t.want(topic.clone()).is_none());
    let ClientMsg::Hello { view, .. } = t.hello() else {
        panic!()
    };
    let delta = |rev, view| ServerMsg::Delta {
        view,
        topic: topic.clone(),
        rev,
        delta: Box::new(TopicDelta::Person(PersonDelta::Counts {
            counts: Counts {
                jar: rev,
                ..Counts::default()
            },
        })),
    };
    // Deltas before the snapshot are ignored.
    assert_eq!(t.receive(delta(1, view)), Outcome::Ignored);
    let snap = ServerMsg::Snapshot {
        view,
        topic: topic.clone(),
        rev: 4,
        state: Box::new(TopicState::Person(Box::new(person(1, 2)))),
    };
    assert_eq!(t.receive(snap.clone()), Outcome::Changed(topic.clone()));
    assert_eq!(t.receive(delta(4, view)), Outcome::Ignored, "duplicate");
    assert_eq!(t.receive(delta(5, view)), Outcome::Changed(topic.clone()));
    let Some(TopicState::Person(p)) = t.state(&topic) else {
        panic!()
    };
    assert_eq!(p.counts.jar, 5);
    // A gap asks for a snapshot and ignores deltas until it arrives.
    assert!(matches!(
        t.receive(delta(7, view)),
        Outcome::Send(ClientMsg::Resync { .. })
    ));
    assert!(!t.fresh(&topic));
    assert_eq!(t.receive(delta(8, view)), Outcome::Ignored);
    // Hiding keeps the view; showing starts a new one, and frames for the old one are dropped.
    assert!(matches!(
        t.set_visible(false),
        Some(ClientMsg::Visibility { visible: false, .. })
    ));
    let Some(ClientMsg::Visibility {
        view: v2,
        visible: true,
    }) = t.set_visible(true)
    else {
        panic!()
    };
    assert!(v2 > view);
    assert_eq!(t.receive(snap), Outcome::Ignored, "old view");
    let snap2 = ServerMsg::Snapshot {
        view: v2,
        topic: topic.clone(),
        rev: 9,
        state: Box::new(TopicState::Person(Box::new(person(1, 2)))),
    };
    assert_eq!(t.receive(snap2), Outcome::Changed(topic.clone()));
    assert!(t.fresh(&topic));
}

#[test]
fn tracker_denied_access_and_broken_frames() {
    let mut t = TopicTracker::new(true);
    t.want(Topic::System);
    t.want(Topic::Wall);
    let view = t.view();
    assert_eq!(
        t.receive(ServerMsg::Denied {
            view,
            topic: Topic::System,
            reason: DenyReason::NotAllowed
        }),
        Outcome::Denied(Topic::System, DenyReason::NotAllowed)
    );
    let Outcome::Retry(again) = t.receive(ServerMsg::AccessChanged) else {
        panic!()
    };
    assert_eq!(
        again,
        vec![ClientMsg::Sub {
            view,
            topic: Topic::System
        }]
    );
    // A state that does not fit the topic is a bug, and the topic is resynced.
    let wrong = ServerMsg::Snapshot {
        view,
        topic: Topic::Wall,
        rev: 1,
        state: Box::new(TopicState::Sidebar(SidebarState::default())),
    };
    assert!(matches!(
        t.receive(wrong),
        Outcome::Broken(Topic::Wall, ClientMsg::Resync { .. })
    ));
    assert_eq!(t.receive(ServerMsg::AuthExpired), Outcome::AuthExpired);
}

/// The bot's cell and a page that joined halfway agree after the same deltas.
#[test]
fn snapshot_plus_deltas_equals_the_cell() {
    let mut cell = person(1, 2);
    let mut deltas = Vec::new();
    for i in 0..120u32 {
        let t = i64::from(i) * 1000;
        deltas.push(PersonDelta::Levels {
            run: LevelRun {
                end_ms: t + 1000,
                frame_ms: 32,
                frames: vec![LevelFrame::new(0.6, -30.0); 31],
            },
        });
        deltas.push(PersonDelta::Sentence {
            card: Box::new(card(i, finished(t))),
        });
        if i % 3 == 0 {
            deltas.push(PersonDelta::Activity {
                item: Activity::Play {
                    id: u64::from(i),
                    kind: pb_domain::PlayPurpose::Warning,
                    at_ms: t,
                    text: None,
                    audience: Audience::Tracked,
                    ended_ms: None,
                    ok: None,
                },
            });
        }
    }
    let half = deltas.len() / 2;
    for d in &deltas[..half] {
        cell.apply(d);
    }
    let mut page = cell.clone();
    for d in &deltas[half..] {
        cell.apply(d);
        page.apply(d);
    }
    assert_eq!(page, cell);
    assert_eq!(cell.sentences.len(), LIVE_SENTENCES);
    assert_eq!(cell.sentences.last().map(|c| c.no), Some(119));
    assert_eq!(cell.activity.len(), LIVE_ACTIVITY);
    let span = cell.levels.end_ms().unwrap_or_default() - cell.levels.runs.front().map_or(0, LevelRun::start_ms);
    assert!(span <= LEVEL_WINDOW_MS && span > LEVEL_WINDOW_MS - 1000, "{span}");
}

#[test]
fn levels_join_runs_and_tell_speaking() {
    let mut l = Levels::default();
    l.push(LevelRun {
        end_ms: 320,
        frame_ms: 32,
        frames: vec![LevelFrame::new(0.1, -60.0); 10],
    });
    l.push(LevelRun {
        end_ms: 384,
        frame_ms: 32,
        frames: vec![LevelFrame::new(0.9, -20.0); 2],
    });
    assert_eq!(l.runs.len(), 1);
    assert!(l.speaking(400, 400));
    assert!(!l.speaking(900, 400));
    l.push(LevelRun {
        end_ms: 2000,
        frame_ms: 32,
        frames: vec![LevelFrame::new(0.0, -90.0); 1],
    });
    assert_eq!(l.runs.len(), 2, "a gap starts a new run");
}

#[test]
fn wall_and_sidebar() {
    let mut w = WallState::default();
    let tile = Tile {
        guild: GuildId(1),
        community: "c".into(),
        who: who(2),
        channel: ChannelRef {
            id: ChannelId(3),
            name: "voice".into(),
        },
        levels: Levels::default(),
        latest: None,
        lag_ms: None,
    };
    w.apply(&WallDelta::Tile {
        tile: Box::new(tile.clone()),
    });
    w.apply(&WallDelta::Sentence {
        guild: GuildId(1),
        user: UserId(2),
        card: Box::new(card(2, finished(0))),
    });
    w.apply(&WallDelta::Sentence {
        guild: GuildId(1),
        user: UserId(2),
        card: Box::new(card(1, finished(0))),
    });
    assert_eq!(
        w.tiles[0].latest.as_ref().map(|c| c.no),
        Some(2),
        "an older sentence does not replace a newer one"
    );
    assert_eq!(w.tiles[0].lag_ms, Some(220));
    w.apply(&WallDelta::Tile { tile: Box::new(tile) });
    assert!(w.tiles[0].latest.is_some(), "updating names keeps the latest sentence");
    w.apply(&WallDelta::TileGone {
        guild: GuildId(1),
        user: UserId(2),
    });
    assert!(w.tiles.is_empty());

    let mut s = SidebarState::default();
    let community = |id: u64, name: &str| SidebarCommunity {
        id: GuildId(id),
        name: name.into(),
        icon: None,
        available: true,
        paused: false,
        people: vec![SidebarPerson {
            who: who(9),
            dot: Dot::Away,
            paused: false,
        }],
    };
    s.apply(&SidebarDelta::Community {
        community: community(2, "b"),
    });
    s.apply(&SidebarDelta::Community {
        community: community(1, "A"),
    });
    assert_eq!(s.communities.iter().map(|c| c.id.0).collect::<Vec<_>>(), vec![1, 2]);
    s.apply(&SidebarDelta::Dot {
        guild: GuildId(2),
        user: UserId(9),
        dot: Dot::Speaking,
    });
    assert_eq!(s.communities[1].people[0].dot, Dot::Speaking);
}

#[test]
fn clock_prefers_the_fastest_round_trip() {
    let mut c = ClockOffset::default();
    assert_eq!(c.server_now(100), 100);
    c.observe(10_000, 1_000, None);
    c.observe(10_500, 1_400, Some(200));
    c.observe(11_000, 1_960, Some(40));
    assert_eq!(c.offset(), 11_000 + 20 - 1_960);
}
