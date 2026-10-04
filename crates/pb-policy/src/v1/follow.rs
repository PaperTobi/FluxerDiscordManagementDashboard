//! Keeps the bot's voice connections matching the channels tracked people are in. A port of the old bot's
//! `follow.py`: no sockets, no clock; callers feed events with `now` (monotonic seconds) and carry out the returned
//! actions, so every transition is testable on a fake clock. One connection per voice channel.
//!
//! ```text
//! SETTLING --settle--> JOINING --grant--> CONNECTING --room up--> CONFIRMING --own voice state seen--> ACTIVE
//!     |                   | no grant in join_timeout   | connect timeout      | no confirmation
//!     |                   v                            v                      v
//!     |                BACKOFF <--------------------------------------------- (leave sent)
//!     `-- not wanted any more: dropped              ACTIVE --not wanted for leave_grace--> LEAVING
//! ```
//!
//! Fluxer facts behind this: refusals send no event (hence the join timeout); a join is pending for 30 s and is only
//! announced when LiveKit reports the participant or the client sends a second op 4 with the same channel and
//! connection (so that confirmation is always sent); leaving needs the connection id; a same-channel op 4 issues no
//! new grant, so recovery after a lost voice connection is leave + a fresh join.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use pb_domain::{ChannelId, ConnectionId, GuildId};

use super::channels::Chan;
use super::world::VState;

const EPS: f64 = 1e-6;

/// Connection states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnState {
    Settling,
    Joining,
    Connecting,
    Confirming,
    Active,
    Leaving,
    Backoff,
}

/// The parts of a voice grant the machine needs (the token stays with the caller).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantInfo {
    pub chan: Chan,
    pub connection: ConnectionId,
    pub has_e2ee_key: bool,
}

/// Kinds of notices for the log and the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    JoinTimeout,
    ConnectFailed,
    ConfirmFailed,
    Involuntary,
    /// Removed again and again: still rejoining, but slower (a moderator, or another instance of the bot?).
    RepeatedRemovals,
    StaleLeave,
    LateGrant,
    E2eeKey,
    /// Someone moved the bot to another channel (Fluxer gives it a new connection there).
    Moved,
}

/// A grant this soon after one of the bot's connections vanished in the same community is that connection moved to
/// another channel: Fluxer ends the old connection and grants a new one where it was moved.
const MOVE_WINDOW_S: f64 = 5.0;

/// What the caller must do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Gateway op 4: join (`channel` set, `connection` = `None`), confirm (both set) or leave (`channel` = `None`).
    VoiceState {
        guild: GuildId,
        channel: Option<ChannelId>,
        connection: Option<ConnectionId>,
    },
    /// Connect to the voice room with the grant received for `connection`.
    Connect { chan: Chan, connection: ConnectionId },
    /// Close the voice room of `chan`.
    Close { chan: Chan },
    Notice {
        kind: NoticeKind,
        text: String,
        chan: Option<Chan>,
    },
}

/// Timings (seconds).
#[derive(Debug, Clone, PartialEq)]
pub struct FollowCfg {
    pub settle_s: f64,
    pub leave_grace_s: f64,
    pub join_timeout_s: f64,
    pub connect_timeout_s: f64,
    pub confirm_resend_s: f64,
    pub confirm_abort_s: f64,
    pub leaving_wait_s: f64,
    /// Waits before retrying after the second failure, the third … (the last one repeats); the first failure only
    /// waits `settle_s`.
    pub backoff_s: Vec<f64>,
    /// After being removed from voice by someone else the bot always joins again: at once the first time, then after
    /// these waits for each further removal within `removal_window_s` (the last one repeats), so a moderator who keeps
    /// removing it, or another instance of it, does not make it flap.
    pub rejoin_s: Vec<f64>,
    pub removal_window_s: f64,
    /// From this many removals within the window the log warns (to keep the bot out: pause the community).
    pub removal_warn: usize,
}

impl Default for FollowCfg {
    fn default() -> Self {
        FollowCfg {
            settle_s: 1.5,
            leave_grace_s: 5.0,
            join_timeout_s: 10.0,
            connect_timeout_s: 12.0,
            confirm_resend_s: 3.0,
            confirm_abort_s: 8.0,
            leaving_wait_s: 3.0,
            backoff_s: vec![15.0, 30.0, 60.0, 120.0, 300.0],
            rejoin_s: vec![5.0, 15.0, 30.0, 60.0],
            removal_window_s: 600.0,
            removal_warn: 3,
        }
    }
}

/// One voice connection.
#[derive(Debug, Clone, PartialEq)]
pub struct Conn {
    pub chan: Chan,
    pub state: ConnState,
    deadline: Option<f64>,
    pub connection: Option<ConnectionId>,
    seen_own: bool,
    undesired_since: Option<f64>,
    abort_on_grant: bool,
    confirm_at: Option<f64>,
    confirm_resent: bool,
    has_room: bool,
}

impl Conn {
    pub fn deadline(&self) -> Option<f64> {
        self.deadline
    }
    pub fn undesired_since(&self) -> Option<f64> {
        self.undesired_since
    }
    pub fn seen_own(&self) -> bool {
        self.seen_own
    }

    fn new(chan: Chan, state: ConnState, deadline: f64) -> Self {
        Conn {
            chan,
            state,
            deadline: Some(deadline),
            connection: None,
            seen_own: false,
            undesired_since: None,
            abort_on_grant: false,
            confirm_at: None,
            confirm_resent: false,
            has_room: false,
        }
    }
}

/// The machine.
#[derive(Debug, Clone)]
pub struct FollowMachine {
    pub cfg: FollowCfg,
    conns: BTreeMap<Chan, Conn>,
    /// Joins wait until the voice world is trustworthy and the model is ready.
    pub enabled: bool,
    /// An op 4 sent while the gateway reconnects is lost, so joins wait for it.
    pub gateway_ok: bool,
    stopping: bool,
    failures: BTreeMap<Chan, usize>,
    involuntary: BTreeMap<GuildId, VecDeque<f64>>,
    paused_until: BTreeMap<GuildId, f64>,
    stale_left: BTreeMap<(GuildId, ConnectionId), f64>,
    /// The channels wanted at the last reconcile.
    desired: BTreeSet<Chan>,
    /// When one of the bot's connections last vanished, per community (to recognise a move in the grant after it).
    vanished: BTreeMap<GuildId, f64>,
}

impl Default for FollowMachine {
    fn default() -> Self {
        FollowMachine::new(FollowCfg::default())
    }
}

fn vs(guild: GuildId, channel: Option<ChannelId>, connection: Option<ConnectionId>) -> Action {
    Action::VoiceState {
        guild,
        channel,
        connection,
    }
}

fn notice(kind: NoticeKind, text: String, chan: Option<Chan>) -> Action {
    Action::Notice { kind, text, chan }
}

impl FollowMachine {
    pub fn new(cfg: FollowCfg) -> Self {
        FollowMachine {
            cfg,
            conns: BTreeMap::new(),
            enabled: false,
            gateway_ok: true,
            stopping: false,
            failures: BTreeMap::new(),
            involuntary: BTreeMap::new(),
            paused_until: BTreeMap::new(),
            stale_left: BTreeMap::new(),
            desired: BTreeSet::new(),
            vanished: BTreeMap::new(),
        }
    }

    /// Joins again in a community where repeated removals paused it (an admin asked for it).
    pub fn resume(&mut self, guild: GuildId) {
        self.paused_until.remove(&guild);
        self.involuntary.remove(&guild);
    }

    pub fn conns(&self) -> impl Iterator<Item = &Conn> {
        self.conns.values()
    }

    pub fn conn(&self, chan: Chan) -> Option<&Conn> {
        self.conns.get(&chan)
    }

    pub fn known_connections(&self) -> BTreeSet<(GuildId, ConnectionId)> {
        self.conns
            .values()
            .filter_map(|c| c.connection.clone().map(|id| (c.chan.guild, id)))
            .collect()
    }

    /// Communities where joining is paused after repeated removals, and until when.
    pub fn paused(&self) -> &BTreeMap<GuildId, f64> {
        &self.paused_until
    }

    /// When the next timer is due (for the caller's sleep).
    pub fn next_deadline(&self) -> Option<f64> {
        let mut times: Vec<f64> = Vec::new();
        for c in self.conns.values() {
            times.extend(c.deadline);
            if let (Some(t), false) = (c.confirm_at, c.confirm_resent) {
                times.push(t);
            }
            if let (Some(since), ConnState::Active) = (c.undesired_since, c.state) {
                times.push(since + self.cfg.leave_grace_s);
            }
        }
        times.extend(self.paused_until.values());
        times.into_iter().reduce(f64::min)
    }

    /// Whenever the world, a timer or the gateway changed. `own`: the bot's own voice states, (guild, connection) →
    /// channel. `stale`: own states left from a previous session (left once). `count_fights`: false while the network
    /// was unstable (our voice state then disappears for reasons that are not a moderator). `unavailable`:
    /// communities whose voice states are unknown, not gone.
    pub fn reconcile(
        &mut self,
        now: f64,
        desired: &[Chan],
        own: &BTreeMap<(GuildId, ConnectionId), ChannelId>,
        stale: &[VState],
        count_fights: bool,
        unavailable: &BTreeSet<GuildId>,
    ) -> Vec<Action> {
        let mut acts = Vec::new();
        if self.stopping {
            return acts;
        }
        let desired_set: BTreeSet<Chan> = desired.iter().copied().collect();
        self.desired.clone_from(&desired_set);
        self.vanished.retain(|_, t| now - *t <= MOVE_WINDOW_S);
        let known = self.known_connections();
        for v in stale {
            let key = v.key();
            if !self.stale_left.contains_key(&key) && !known.contains(&key) {
                self.stale_left.insert(key, now);
                acts.push(vs(v.guild, None, Some(v.connection.clone())));
                acts.push(notice(
                    NoticeKind::StaleLeave,
                    format!(
                        "leaving a stale bot connection from a previous session in {}/{}",
                        v.guild, v.channel
                    ),
                    Some(Chan {
                        guild: v.guild,
                        channel: v.channel,
                    }),
                ));
            }
        }

        let chans: Vec<Chan> = self.conns.keys().copied().collect();
        for chan in chans {
            let Some(conn) = self.conns.get(&chan) else { continue };
            let Some(id) = conn.connection.clone() else { continue };
            if !matches!(
                conn.state,
                ConnState::Connecting | ConnState::Confirming | ConnState::Active
            ) || unavailable.contains(&chan.guild)
            {
                continue;
            }
            match own.get(&(chan.guild, id)) {
                Some(channel) if *channel != chan.channel => {
                    acts.extend(self.involuntary_removal(
                        chan,
                        now,
                        format!("was moved to channel {channel} by someone else"),
                        count_fights,
                    ));
                }
                Some(_) => {
                    let conn = self.conns.get_mut(&chan).unwrap_or_else(|| unreachable!());
                    conn.seen_own = true;
                    if conn.state == ConnState::Confirming {
                        self.become_active(chan);
                    }
                }
                None if conn.seen_own => {
                    self.vanished.insert(chan.guild, now);
                    acts.extend(self.involuntary_removal(
                        chan,
                        now,
                        "its voice state disappeared (disconnected by a moderator or the server)".into(),
                        count_fights,
                    ));
                }
                None => {}
            }
        }

        let chans: Vec<Chan> = self.conns.keys().copied().collect();
        for chan in chans {
            if desired_set.contains(&chan) {
                if let Some(conn) = self.conns.get_mut(&chan) {
                    conn.undesired_since = None;
                    conn.abort_on_grant = false;
                }
                continue;
            }
            acts.extend(self.not_wanted(chan, now));
        }

        if self.enabled && !self.stopping {
            for chan in desired {
                if self.conns.contains_key(chan) {
                    continue;
                }
                if self.paused_until.get(&chan.guild).is_some_and(|until| *until > now) {
                    continue;
                }
                let n = self.failures.get(chan).copied().unwrap_or(0);
                let (state, delay) = if n <= 1 {
                    (ConnState::Settling, self.cfg.settle_s)
                } else {
                    let i = (n - 2).min(self.cfg.backoff_s.len().saturating_sub(1));
                    (ConnState::Backoff, self.cfg.backoff_s.get(i).copied().unwrap_or(0.0))
                };
                self.conns.insert(*chan, Conn::new(*chan, state, now + delay));
            }
        }
        let conns = &self.conns;
        self.failures
            .retain(|chan, _| desired_set.contains(chan) || conns.contains_key(chan));
        self.paused_until.retain(|_, until| *until > now + EPS);
        acts
    }

    /// Timers.
    pub fn on_tick(&mut self, now: f64) -> Vec<Action> {
        let mut acts = Vec::new();
        if self.stopping {
            return acts;
        }
        let chans: Vec<Chan> = self.conns.keys().copied().collect();
        for chan in chans {
            let Some(conn) = self.conns.get_mut(&chan) else {
                continue;
            };
            if conn.state == ConnState::Active
                && conn
                    .undesired_since
                    .is_some_and(|since| now - since >= self.cfg.leave_grace_s - EPS)
            {
                acts.extend(self.begin_leave(chan, now));
                continue;
            }
            if conn.state == ConnState::Confirming
                && !conn.confirm_resent
                && conn.confirm_at.is_some_and(|t| now >= t - EPS)
            {
                conn.confirm_resent = true;
                acts.push(vs(chan.guild, Some(chan.channel), conn.connection.clone()));
            }
            let Some(deadline) = conn.deadline else { continue };
            if now < deadline - EPS {
                continue;
            }
            match conn.state {
                ConnState::Settling | ConnState::Backoff => {
                    if !self.gateway_ok {
                        conn.deadline = Some(now + 0.5);
                        continue;
                    }
                    conn.state = ConnState::Joining;
                    conn.deadline = Some(now + self.cfg.join_timeout_s);
                    acts.push(vs(chan.guild, Some(chan.channel), None));
                }
                ConnState::Joining => {
                    let text = format!(
                        "no voice grant for {chan} within {:.0}s. Fluxer refuses silently; check the bot has View Channel and Connect in that \
                         channel (and Speak to talk), the channel is not full and the bot is not timed out",
                        self.cfg.join_timeout_s
                    );
                    acts.extend(self.fail(chan, NoticeKind::JoinTimeout, text));
                }
                ConnState::Connecting => {
                    let text = format!(
                        "the voice connection for {chan} did not come up in {:.0}s",
                        self.cfg.connect_timeout_s
                    );
                    acts.extend(self.fail(chan, NoticeKind::ConnectFailed, text));
                }
                ConnState::Confirming => {
                    let text = format!("{chan}: Fluxer never announced our voice state; leaving and retrying");
                    acts.extend(self.fail(chan, NoticeKind::ConfirmFailed, text));
                }
                ConnState::Leaving => {
                    self.conns.remove(&chan);
                }
                ConnState::Active => {}
            }
        }
        self.paused_until.retain(|_, until| *until > now + EPS);
        acts
    }

    /// A voice grant arrived (VOICE_SERVER_UPDATE).
    pub fn on_grant(&mut self, now: f64, grant: &GrantInfo) -> Vec<Action> {
        let chan = grant.chan;
        let mut acts = Vec::new();
        if grant.has_e2ee_key {
            acts.push(notice(
                NoticeKind::E2eeKey,
                format!("{chan}: the grant carries an end-to-end encryption key although this bot never claims to support it; ignoring it"),
                Some(chan),
            ));
        }
        let leave = |acts: &mut Vec<Action>| acts.push(vs(chan.guild, None, Some(grant.connection.clone())));
        let waiting = self
            .conns
            .get(&chan)
            .is_none_or(|c| matches!(c.state, ConnState::Settling | ConnState::Backoff));
        let moved = self.vanished.get(&chan.guild).is_some_and(|t| now - t <= MOVE_WINDOW_S);
        if waiting && moved && !self.stopping {
            return self.moved(now, grant);
        }
        let Some(conn) = self.conns.get_mut(&chan) else {
            leave(&mut acts);
            acts.push(notice(
                NoticeKind::LateGrant,
                format!(
                    "{chan}: grant {} arrived with no pending join; leaving it",
                    grant.connection
                ),
                Some(chan),
            ));
            return acts;
        };
        if matches!(
            conn.state,
            ConnState::Settling | ConnState::Backoff | ConnState::Leaving
        ) || self.stopping
        {
            leave(&mut acts);
            acts.push(notice(
                NoticeKind::LateGrant,
                format!(
                    "{chan}: grant {} arrived with no pending join; leaving it",
                    grant.connection
                ),
                Some(chan),
            ));
            return acts;
        }
        if conn.connection.as_ref().is_some_and(|c| *c != grant.connection) {
            leave(&mut acts);
            return acts;
        }
        if conn.state == ConnState::Joining && conn.abort_on_grant {
            self.conns.remove(&chan);
            leave(&mut acts);
            return acts;
        }
        let was_active_like = matches!(
            conn.state,
            ConnState::Connecting | ConnState::Confirming | ConnState::Active
        );
        conn.connection = Some(grant.connection.clone());
        conn.state = ConnState::Connecting;
        conn.deadline = Some(now + self.cfg.connect_timeout_s);
        conn.confirm_at = None;
        conn.confirm_resent = false;
        if !was_active_like {
            conn.seen_own = false;
        }
        conn.has_room = true;
        acts.push(Action::Connect {
            chan,
            connection: grant.connection.clone(),
        });
        acts
    }

    /// The voice room of `chan` is connected.
    pub fn on_room_up(&mut self, now: f64, chan: Chan) -> Vec<Action> {
        let Some(conn) = self.conns.get_mut(&chan) else {
            return Vec::new();
        };
        if conn.state != ConnState::Connecting {
            return Vec::new();
        }
        if conn.seen_own {
            self.become_active(chan);
            return Vec::new();
        }
        conn.state = ConnState::Confirming;
        conn.deadline = Some(now + self.cfg.confirm_abort_s);
        conn.confirm_at = Some(now + self.cfg.confirm_resend_s);
        conn.confirm_resent = false;
        vec![vs(chan.guild, Some(chan.channel), conn.connection.clone())]
    }

    /// The voice room of `chan` was lost.
    pub fn on_room_down(&mut self, _now: f64, chan: Chan, reason: &str) -> Vec<Action> {
        let Some(conn) = self.conns.get(&chan) else {
            return Vec::new();
        };
        if !matches!(
            conn.state,
            ConnState::Connecting | ConnState::Confirming | ConnState::Active
        ) {
            return Vec::new();
        }
        let kind = if conn.state == ConnState::Active {
            NoticeKind::Involuntary
        } else {
            NoticeKind::ConnectFailed
        };
        let reason = if reason.is_empty() { "no reason given" } else { reason };
        self.fail(
            chan,
            kind,
            format!("the voice connection for {chan} was lost ({reason}); rejoining with a fresh grant"),
        )
    }

    /// A brand-new gateway session (not a resume): the old session's voice connections are gone.
    pub fn on_gateway_fresh(&mut self, _now: f64) -> Vec<Action> {
        let acts = self
            .conns
            .values()
            .filter(|c| c.has_room)
            .map(|c| Action::Close { chan: c.chan })
            .collect();
        self.conns.clear();
        self.stale_left.clear();
        acts
    }

    pub fn shutdown(&mut self, _now: f64) -> Vec<Action> {
        self.stopping = true;
        let mut acts = Vec::new();
        for c in self.conns.values() {
            if c.has_room {
                acts.push(Action::Close { chan: c.chan });
            }
            if let Some(id) = &c.connection {
                acts.push(vs(c.chan.guild, None, Some(id.clone())));
            }
        }
        self.conns.clear();
        acts
    }

    fn become_active(&mut self, chan: Chan) {
        if let Some(c) = self.conns.get_mut(&chan) {
            c.state = ConnState::Active;
            c.deadline = None;
            c.confirm_at = None;
        }
        self.failures.remove(&chan);
    }

    fn not_wanted(&mut self, chan: Chan, now: f64) -> Vec<Action> {
        let Some(conn) = self.conns.get_mut(&chan) else {
            return Vec::new();
        };
        match conn.state {
            ConnState::Settling | ConnState::Backoff => {
                self.conns.remove(&chan);
                Vec::new()
            }
            ConnState::Joining => {
                conn.abort_on_grant = true;
                Vec::new()
            }
            ConnState::Connecting | ConnState::Confirming => self.begin_leave(chan, now),
            ConnState::Active => {
                let since = *conn.undesired_since.get_or_insert(now);
                if now - since >= self.cfg.leave_grace_s - EPS {
                    self.begin_leave(chan, now)
                } else {
                    Vec::new()
                }
            }
            ConnState::Leaving => Vec::new(),
        }
    }

    fn begin_leave(&mut self, chan: Chan, now: f64) -> Vec<Action> {
        let Some(conn) = self.conns.get_mut(&chan) else {
            return Vec::new();
        };
        let mut acts = Vec::new();
        if conn.has_room {
            acts.push(Action::Close { chan });
            conn.has_room = false;
        }
        if let Some(id) = &conn.connection {
            acts.push(vs(chan.guild, None, Some(id.clone())));
        }
        conn.state = ConnState::Leaving;
        conn.deadline = Some(now + self.cfg.leaving_wait_s);
        conn.confirm_at = None;
        conn.undesired_since = None;
        acts
    }

    /// Forgets a connection at once: its room is closed and its voice state left.
    fn drop_conn(&mut self, chan: Chan, acts: &mut Vec<Action>) {
        if let Some(conn) = self.conns.remove(&chan) {
            if conn.has_room {
                acts.push(Action::Close { chan });
            }
            if let Some(id) = conn.connection {
                acts.push(vs(chan.guild, None, Some(id)));
            }
        }
    }

    fn fail(&mut self, chan: Chan, kind: NoticeKind, text: String) -> Vec<Action> {
        let mut acts = vec![notice(kind, text, Some(chan))];
        self.drop_conn(chan, &mut acts);
        *self.failures.entry(chan).or_insert(0) += 1;
        acts
    }

    /// The grant of a connection someone moved the bot to. Where a followed person is, the bot stays (that was no
    /// removal); anywhere else it leaves and goes back to them (the move counts as a removal).
    fn moved(&mut self, now: f64, grant: &GrantInfo) -> Vec<Action> {
        let chan = grant.chan;
        let since = self.vanished.remove(&chan.guild).unwrap_or(now);
        if !self.desired.contains(&chan) {
            return vec![
                vs(chan.guild, None, Some(grant.connection.clone())),
                notice(
                    NoticeKind::Moved,
                    format!("{chan}: someone moved the bot here; nobody it follows is here, so it goes back"),
                    Some(chan),
                ),
            ];
        }
        // Not a removal after all: take back its count (and the pause it may have caused).
        if let Some(hist) = self.involuntary.get_mut(&chan.guild) {
            hist.pop_back();
        }
        if self
            .paused_until
            .get(&chan.guild)
            .is_some_and(|until| *until > since - EPS)
        {
            self.paused_until.remove(&chan.guild);
        }
        let mut conn = Conn::new(chan, ConnState::Connecting, now + self.cfg.connect_timeout_s);
        conn.connection = Some(grant.connection.clone());
        conn.has_room = true;
        self.conns.insert(chan, conn);
        vec![
            notice(
                NoticeKind::Moved,
                format!("{chan}: someone moved the bot here, where a person it follows is; it stays"),
                Some(chan),
            ),
            Action::Connect {
                chan,
                connection: grant.connection.clone(),
            },
        ]
    }

    fn involuntary_removal(&mut self, chan: Chan, now: f64, why: String, count_fights: bool) -> Vec<Action> {
        let mut acts = vec![notice(
            NoticeKind::Involuntary,
            format!("{chan}: our voice connection {why}"),
            Some(chan),
        )];
        self.drop_conn(chan, &mut acts);
        if !count_fights {
            return acts;
        }
        let hist = self.involuntary.entry(chan.guild).or_default();
        hist.push_back(now);
        while hist.front().is_some_and(|t| now - t > self.cfg.removal_window_s) {
            hist.pop_front();
        }
        let n = hist.len();
        // The first removal: back at once (after the usual settle); every further one waits a little longer.
        let Some(wait) = n
            .checked_sub(2)
            .and_then(|i| self.cfg.rejoin_s.get(i.min(self.cfg.rejoin_s.len().saturating_sub(1))))
            .copied()
        else {
            return acts;
        };
        self.paused_until.insert(chan.guild, now + wait);
        if n == self.cfg.removal_warn {
            acts.push(notice(
                NoticeKind::RepeatedRemovals,
                format!(
                    "removed {n}x within {:.0} min in community {} (a moderator, or another instance of this bot?); it keeps joining again, waiting longer each time (now {wait:.0} s). To keep it out, pause it there",
                    self.cfg.removal_window_s / 60.0,
                    chan.guild,
                ),
                Some(chan),
            ));
        }
        acts
    }
}
