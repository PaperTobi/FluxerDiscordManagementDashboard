//! The old bot's FollowMachine tests (tests/unit/test_follow.py), ported: every transition on a fake clock, plus a
//! fuzz run asserting the invariants.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::collections::{BTreeMap, BTreeSet};

use pb_domain::{ChannelId, ConnectionId, GuildId, UserId};
use pb_policy::{Action, Chan, ConnState, FollowCfg, FollowMachine, GrantInfo, VState};

const A: Chan = Chan {
    guild: GuildId(1),
    channel: ChannelId(10),
};
const B: Chan = Chan {
    guild: GuildId(1),
    channel: ChannelId(11),
};
const G2: Chan = Chan {
    guild: GuildId(2),
    channel: ChannelId(20),
};

fn c(id: &str) -> ConnectionId {
    ConnectionId(id.into())
}

fn grant(chan: Chan, conn: &str) -> GrantInfo {
    GrantInfo {
        chan,
        connection: c(conn),
        has_e2ee_key: false,
    }
}

fn join(chan: Chan) -> Action {
    Action::VoiceState {
        guild: chan.guild,
        channel: Some(chan.channel),
        connection: None,
    }
}

fn confirm(chan: Chan, conn: &str) -> Action {
    Action::VoiceState {
        guild: chan.guild,
        channel: Some(chan.channel),
        connection: Some(c(conn)),
    }
}

fn leave(guild: GuildId, conn: &str) -> Action {
    Action::VoiceState {
        guild,
        channel: None,
        connection: Some(c(conn)),
    }
}

fn kinds(acts: &[Action]) -> Vec<String> {
    acts.iter()
        .map(|a| match a {
            Action::VoiceState { .. } => "VoiceState".to_owned(),
            Action::Connect { .. } => "Connect".to_owned(),
            Action::Close { .. } => "Close".to_owned(),
            Action::Notice { kind, .. } => format!(
                "Notice:{}",
                serde_json::to_value(kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            ),
        })
        .collect()
}

fn own(entries: &[(Chan, &str)]) -> BTreeMap<(GuildId, ConnectionId), ChannelId> {
    entries
        .iter()
        .map(|(ch, conn)| ((ch.guild, c(conn)), ch.channel))
        .collect()
}

fn none() -> BTreeSet<GuildId> {
    BTreeSet::new()
}

fn new(cfg: FollowCfg) -> FollowMachine {
    let mut m = FollowMachine::new(cfg);
    m.enabled = true;
    m
}

fn reconcile(
    m: &mut FollowMachine,
    now: f64,
    desired: &[Chan],
    own_states: &BTreeMap<(GuildId, ConnectionId), ChannelId>,
) -> Vec<Action> {
    m.reconcile(now, desired, own_states, &[], true, &none())
}

fn drive_to_active(m: &mut FollowMachine, chan: Chan, t: f64, conn: &str) -> f64 {
    reconcile(m, t, &[chan], &BTreeMap::new());
    let t = t + m.cfg.settle_s;
    assert_eq!(m.on_tick(t), vec![join(chan)]);
    assert_eq!(
        m.on_grant(t + 0.1, &grant(chan, conn)),
        vec![Action::Connect {
            chan,
            connection: c(conn)
        }]
    );
    assert_eq!(m.on_room_up(t + 0.5, chan), vec![confirm(chan, conn)]);
    assert_eq!(m.conn(chan).map(|x| x.state), Some(ConnState::Confirming));
    reconcile(m, t + 0.7, &[chan], &own(&[(chan, conn)]));
    assert_eq!(m.conn(chan).map(|x| x.state), Some(ConnState::Active));
    t + 0.7
}

#[test]
fn happy_path_join_confirm_active() {
    let mut m = new(FollowCfg::default());
    drive_to_active(&mut m, A, 0.0, "c1");
}

#[test]
fn nothing_happens_until_enabled() {
    let mut m = FollowMachine::default();
    assert!(reconcile(&mut m, 0.0, &[A], &BTreeMap::new()).is_empty() && m.conns().next().is_none());
    m.enabled = true;
    reconcile(&mut m, 1.0, &[A], &BTreeMap::new());
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Settling));
}

#[test]
fn drops_when_the_person_leaves_before_the_join() {
    let mut m = new(FollowCfg::default());
    reconcile(&mut m, 0.0, &[A], &BTreeMap::new());
    reconcile(&mut m, 0.5, &[], &BTreeMap::new());
    assert!(m.conns().next().is_none());
    assert!(m.on_tick(5.0).is_empty());
}

#[test]
fn join_timeout_backs_off_with_diagnostics() {
    let mut m = new(FollowCfg::default());
    reconcile(&mut m, 0.0, &[A], &BTreeMap::new());
    m.on_tick(1.5);
    let acts = m.on_tick(1.5 + 10.0);
    assert_eq!(kinds(&acts), vec!["Notice:join_timeout"]);
    assert!(m.conns().next().is_none());
    reconcile(&mut m, 12.0, &[A], &BTreeMap::new());
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Settling));
    m.on_tick(13.5);
    m.on_tick(23.5);
    reconcile(&mut m, 24.0, &[A], &BTreeMap::new());
    let conn = m.conn(A).expect("conn");
    assert_eq!((conn.state, conn.deadline()), (ConnState::Backoff, Some(24.0 + 15.0)));
}

#[test]
fn confirmation_is_resent_then_aborted() {
    let mut m = new(FollowCfg::default());
    reconcile(&mut m, 0.0, &[A], &BTreeMap::new());
    m.on_tick(1.5);
    m.on_grant(1.6, &grant(A, "c1"));
    m.on_room_up(2.0, A);
    assert_eq!(m.on_tick(5.0), vec![confirm(A, "c1")]);
    assert!(m.on_tick(5.1).is_empty());
    let acts = m.on_tick(10.0);
    assert_eq!(kinds(&acts), vec!["Notice:confirm_failed", "Close", "VoiceState"]);
    assert_eq!(acts[2], leave(GuildId(1), "c1"));
}

#[test]
fn own_state_arriving_before_the_room_needs_no_confirmation() {
    let mut m = new(FollowCfg::default());
    reconcile(&mut m, 0.0, &[A], &BTreeMap::new());
    m.on_tick(1.5);
    m.on_grant(1.6, &grant(A, "c1"));
    reconcile(&mut m, 1.7, &[A], &own(&[(A, "c1")]));
    assert!(m.on_room_up(1.8, A).is_empty());
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Active));
}

#[test]
fn connect_timeout_leaves() {
    let mut m = new(FollowCfg::default());
    reconcile(&mut m, 0.0, &[A], &BTreeMap::new());
    m.on_tick(1.5);
    m.on_grant(1.6, &grant(A, "c1"));
    assert_eq!(
        kinds(&m.on_tick(13.6)),
        vec!["Notice:connect_failed", "Close", "VoiceState"]
    );
}

#[test]
fn leaves_after_the_grace_unless_the_person_returns() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c1");
    let o = own(&[(A, "c1")]);
    assert!(reconcile(&mut m, t + 1.0, &[], &o).is_empty());
    assert!(reconcile(&mut m, t + 3.0, &[A], &o).is_empty());
    assert_eq!(m.conn(A).and_then(|x| x.undesired_since()), None);
    reconcile(&mut m, t + 10.0, &[], &o);
    let acts = m.on_tick(t + 15.0);
    assert_eq!(kinds(&acts), vec!["Close", "VoiceState"]);
    assert_eq!(acts[1], leave(GuildId(1), "c1"));
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Leaving));
    m.on_tick(t + 18.0);
    assert!(m.conns().next().is_none());
}

#[test]
fn channel_change_is_leave_plus_fresh_join() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c1");
    let o = own(&[(A, "c1")]);
    reconcile(&mut m, t + 1.0, &[B], &o);
    assert_eq!(m.conn(B).map(|x| x.state), Some(ConnState::Settling));
    assert!(m.conn(A).and_then(|x| x.undesired_since()).is_some());
    let t2 = t + 6.0;
    assert_eq!(kinds(&reconcile(&mut m, t2, &[B], &o)), vec!["Close", "VoiceState"]);
    assert_eq!(m.on_tick(t2 + 0.1), vec![join(B)]);
}

#[test]
fn two_channels_two_connections_across_communities() {
    let mut m = new(FollowCfg::default());
    reconcile(&mut m, 0.0, &[A, G2], &BTreeMap::new());
    let acts = m.on_tick(1.5);
    assert_eq!(acts.len(), 2);
    assert!(acts.iter().all(|a| matches!(
        a,
        Action::VoiceState {
            connection: None,
            channel: Some(_),
            ..
        }
    )));
    assert_eq!(
        m.on_grant(1.6, &grant(G2, "g2c")),
        vec![Action::Connect {
            chan: G2,
            connection: c("g2c")
        }]
    );
    assert_eq!(
        m.on_grant(1.7, &grant(A, "c1")),
        vec![Action::Connect {
            chan: A,
            connection: c("c1")
        }]
    );
}

#[test]
fn repeated_removals_slow_the_rejoin_down_but_never_stop_it() {
    let mut m = new(FollowCfg {
        rejoin_s: vec![5.0, 15.0],
        removal_warn: 3,
        ..FollowCfg::default()
    });
    let mut t = 0.0;
    for i in 0..5 {
        t = drive_to_active(&mut m, A, t, &format!("c{i}")) + 1.0;
        let acts = reconcile(&mut m, t, &[A], &BTreeMap::new());
        assert!(kinds(&acts).contains(&"Notice:involuntary".to_owned()));
        assert_eq!(
            kinds(&acts).contains(&"Notice:repeated_removals".to_owned()),
            i == 2,
            "warned once, at 3"
        );
        let wait = [0.0, 5.0, 15.0, 15.0, 15.0][i];
        if wait > 0.0 {
            reconcile(&mut m, t + wait - 1.0, &[A], &BTreeMap::new());
            assert!(m.conn(A).is_none(), "removal {}: still waiting", i + 1);
        }
        m.on_tick(t + wait);
        reconcile(&mut m, t + wait, &[A], &BTreeMap::new());
        assert!(m.conn(A).is_some(), "removal {}: joining again", i + 1);
        t += wait;
    }
}

#[test]
fn being_moved_counts_as_a_removal() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c1");
    let moved: BTreeMap<_, _> = [((GuildId(1), c("c1")), ChannelId(99))].into();
    let acts = reconcile(&mut m, t + 1.0, &[A], &moved);
    assert!(kinds(&acts).contains(&"Notice:involuntary".to_owned()) && acts.contains(&leave(GuildId(1), "c1")));
}

#[test]
fn a_lost_room_rejoins_with_a_fresh_grant() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c1");
    assert_eq!(
        kinds(&m.on_room_down(t + 1.0, A, "server closed")),
        vec!["Notice:involuntary", "Close", "VoiceState"]
    );
    reconcile(&mut m, t + 1.1, &[A], &own(&[(A, "c1")]));
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Settling));
}

#[test]
fn late_or_unknown_grants_are_left_not_leaked() {
    let mut m = new(FollowCfg::default());
    assert!(m.on_grant(0.0, &grant(A, "zzz")).contains(&leave(GuildId(1), "zzz")));
    reconcile(&mut m, 1.0, &[A], &BTreeMap::new());
    m.on_tick(2.5);
    reconcile(&mut m, 3.0, &[], &BTreeMap::new());
    assert_eq!(m.on_grant(3.5, &grant(A, "c9")), vec![leave(GuildId(1), "c9")]);
    assert!(m.conn(A).is_none());
}

#[test]
fn a_second_grant_for_another_connection_is_left() {
    let mut m = new(FollowCfg::default());
    reconcile(&mut m, 0.0, &[A], &BTreeMap::new());
    m.on_tick(1.5);
    m.on_grant(1.6, &grant(A, "c1"));
    assert_eq!(m.on_grant(1.7, &grant(A, "other")), vec![leave(GuildId(1), "other")]);
}

#[test]
fn a_region_change_regrant_keeps_the_connection() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c1");
    assert_eq!(
        m.on_grant(t + 1.0, &grant(A, "c1")),
        vec![Action::Connect {
            chan: A,
            connection: c("c1")
        }]
    );
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Connecting));
    assert!(m.conn(A).is_some_and(|x| x.seen_own()));
    assert!(m.on_room_up(t + 2.0, A).is_empty());
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Active));
}

#[test]
fn a_fresh_gateway_session_drops_everything_and_stale_states_are_left_once() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c1");
    assert_eq!(m.on_gateway_fresh(t + 1.0), vec![Action::Close { chan: A }]);
    assert!(m.conns().next().is_none());
    let stale = vec![VState {
        guild: GuildId(1),
        channel: ChannelId(10),
        user: UserId(999),
        connection: c("old"),
        session: Some("S0".into()),
        e2ee_capable: false,
        self_mute: false,
        mute: false,
        deaf: false,
        version: 0,
        seq: 1,
    }];
    let acts = m.reconcile(t + 2.0, &[], &BTreeMap::new(), &stale, true, &none());
    assert_eq!(acts[0], leave(GuildId(1), "old"));
    assert!(
        m.reconcile(t + 3.0, &[], &BTreeMap::new(), &stale, true, &none())
            .is_empty()
    );
}

#[test]
fn shutdown_leaves_every_connection_and_blocks_new_work() {
    let mut m = new(FollowCfg::default());
    drive_to_active(&mut m, A, 0.0, "c1");
    assert_eq!(kinds(&m.shutdown(100.0)), vec!["Close", "VoiceState"]);
    assert!(reconcile(&mut m, 101.0, &[A], &BTreeMap::new()).is_empty() && m.conns().next().is_none());
}

#[test]
fn next_deadline_tracks_timers() {
    let mut m = new(FollowCfg::default());
    assert_eq!(m.next_deadline(), None);
    reconcile(&mut m, 0.0, &[A], &BTreeMap::new());
    assert_eq!(m.next_deadline(), Some(1.5));
}

#[test]
fn a_down_gateway_defers_the_join() {
    let mut m = new(FollowCfg::default());
    m.gateway_ok = false;
    reconcile(&mut m, 0.0, &[A], &BTreeMap::new());
    assert!(m.on_tick(1.5).is_empty());
    assert_eq!(
        m.conn(A).map(|x| (x.state, x.deadline())),
        Some((ConnState::Settling, Some(2.0)))
    );
    m.gateway_ok = true;
    assert_eq!(m.on_tick(2.0), vec![join(A)]);
}

#[test]
fn removals_during_network_trouble_never_pause() {
    let mut m = new(FollowCfg::default());
    let mut t = 0.0;
    for i in 0..5 {
        t = drive_to_active(&mut m, A, t, &format!("c{i}")) + 1.0;
        let acts = m.reconcile(t, &[A], &BTreeMap::new(), &[], false, &none());
        assert!(
            kinds(&acts).contains(&"Notice:involuntary".to_owned())
                && !kinds(&acts).contains(&"Notice:repeated_removals".to_owned())
        );
        assert!(m.paused().is_empty(), "no waiting either");
        t += 1.0;
    }
    assert!(m.conn(A).is_some() && m.paused().is_empty());
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Settling));
}

#[test]
fn an_unavailable_community_is_not_a_removal() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c1");
    let down: BTreeSet<GuildId> = [GuildId(1)].into();
    assert!(m.reconcile(t + 1.0, &[], &BTreeMap::new(), &[], true, &down).is_empty());
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Active));
    assert!(reconcile(&mut m, t + 2.0, &[A], &own(&[(A, "c1")])).is_empty());
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Active));
}

/// A small deterministic random source for the fuzz run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[test]
fn fuzz_invariants() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let chans = [A, B, G2];
    for _run in 0..200 {
        let settle = if rng.next() < 0.5 { 0.0 } else { 1.5 };
        let mut m = new(FollowCfg {
            settle_s: settle,
            ..FollowCfg::default()
        });
        let (mut now, mut n) = (0.0, 0);
        for _step in 0..60 {
            now += rng.next() * 4.0;
            let desired: Vec<Chan> = chans.iter().copied().filter(|_| rng.next() < 0.5).collect();
            let mut own_states = BTreeMap::new();
            for conn in m.conns() {
                if let Some(id) = &conn.connection
                    && rng.next() < 0.7
                {
                    let ch = if rng.next() < 0.95 {
                        conn.chan.channel
                    } else {
                        ChannelId(99)
                    };
                    own_states.insert((conn.chan.guild, id.clone()), ch);
                }
            }
            let op = rng.next();
            let acts = if op < 0.4 {
                reconcile(&mut m, now, &desired, &own_states)
            } else if op < 0.6 {
                m.on_tick(now)
            } else if op < 0.75 {
                let joining: Vec<Chan> = m
                    .conns()
                    .filter(|x| x.state == ConnState::Joining)
                    .map(|x| x.chan)
                    .collect();
                let mut acts = Vec::new();
                for ch in joining {
                    n += 1;
                    acts.extend(m.on_grant(now, &grant(ch, &format!("x{n}"))));
                }
                acts
            } else if op < 0.85 {
                let ch = chans[(rng.next() * 3.0) as usize % 3];
                m.on_room_up(now, ch)
            } else if op < 0.92 {
                let ch = chans[(rng.next() * 3.0) as usize % 3];
                m.on_room_down(now, ch, "fuzz")
            } else if op < 0.95 {
                m.on_gateway_fresh(now)
            } else {
                let ch = chans[(rng.next() * 3.0) as usize % 3];
                m.on_grant(now, &grant(ch, "stray"))
            };
            for a in &acts {
                if let Action::VoiceState {
                    guild,
                    channel: Some(ch),
                    connection: None,
                } = a
                {
                    let conn = m.conn(Chan {
                        guild: *guild,
                        channel: *ch,
                    });
                    assert!(
                        conn.is_some_and(|x| x.state == ConnState::Joining),
                        "join sent without a JOINING connection"
                    );
                }
                if let Action::VoiceState {
                    channel: None,
                    connection,
                    ..
                } = a
                {
                    assert!(connection.is_some(), "a leave must carry the connection id");
                }
            }
            for conn in m.conns() {
                if matches!(
                    conn.state,
                    ConnState::Connecting | ConnState::Confirming | ConnState::Active
                ) {
                    assert!(conn.connection.is_some());
                }
            }
        }
        m.shutdown(now + 1.0);
        assert!(m.conns().next().is_none());
    }
}

#[test]
fn moved_where_the_person_is_it_stays_and_that_is_no_removal() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c0") + 1.0;
    // A moderator moved the person and the bot to B: the bot's connection in A is gone, a new one is granted in B.
    let acts = reconcile(&mut m, t, &[B], &BTreeMap::new());
    assert!(kinds(&acts).contains(&"Notice:involuntary".to_owned()));
    let acts = m.on_grant(t + 0.01, &grant(B, "c1"));
    assert_eq!(kinds(&acts), vec!["Notice:moved", "Connect"]);
    assert_eq!(m.conn(B).map(|x| x.state), Some(ConnState::Connecting));
    assert!(m.paused().is_empty(), "a move to the person is no removal");
}

#[test]
fn moved_away_from_the_person_it_goes_back() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c0") + 1.0;
    reconcile(&mut m, t, &[A], &BTreeMap::new());
    let acts = m.on_grant(t + 0.01, &grant(B, "c1"));
    assert_eq!(acts[0], leave(B.guild, "c1"));
    assert_eq!(kinds(&acts), vec!["VoiceState", "Notice:moved"]);
    assert_eq!(
        m.conn(A).map(|x| x.state),
        Some(ConnState::Settling),
        "on its way back to A"
    );
}

#[test]
fn a_grant_long_after_a_removal_is_not_a_move() {
    let mut m = new(FollowCfg::default());
    let t = drive_to_active(&mut m, A, 0.0, "c0") + 1.0;
    reconcile(&mut m, t, &[A], &BTreeMap::new());
    reconcile(&mut m, t + 10.0, &[A], &BTreeMap::new());
    let acts = m.on_grant(t + 10.0, &grant(B, "c1"));
    assert_eq!(kinds(&acts), vec!["VoiceState", "Notice:late_grant"]);
}

#[test]
fn an_admin_can_end_the_wait_after_repeated_removals() {
    let mut m = new(FollowCfg::default());
    let mut t = 0.0;
    for i in 0..2 {
        t = drive_to_active(&mut m, A, t, &format!("c{i}")) + 1.0;
        reconcile(&mut m, t, &[A], &BTreeMap::new());
        t += 1.0;
    }
    assert!(m.paused().contains_key(&A.guild), "waiting after the second removal");
    m.resume(A.guild);
    reconcile(&mut m, t + 1.0, &[A], &BTreeMap::new());
    assert_eq!(m.conn(A).map(|x| x.state), Some(ConnState::Settling));
}
