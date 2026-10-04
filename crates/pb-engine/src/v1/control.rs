//! The control actor of one Fluxer session: the voice world, the community burst after login, the follow machine
//! (when to join, confirm, connect, leave), the rooms, chat commands and the bot's custom status.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use pb_domain::{ChannelId, ConnectionId, GuildId, UserId};
use pb_fluxer_api::{Fatal, FluxerCtl, GatewayEvent, VoiceGrant, VoiceStateOp};
use pb_live_proto::Dot;
use pb_policy::{Action, Burst, Chan, FollowCfg, FollowMachine, GrantInfo, desired_channels, stale_own_states};
use pb_store_api::{CommunitySeen, Event, PersonSeen};
use tokio::sync::mpsc;

use super::core::{Core, RoomCmd};

/// After the gateway was down or resumed, a vanished own voice state is not counted as a moderator's removal.
const UNSTABLE_S: f64 = 90.0;
/// The community burst after login is complete when all communities answered, or after this long.
const BURST_CAP_S: f64 = 10.0;
const TICK: Duration = Duration::from_millis(250);

/// Messages from rooms.
#[derive(Debug)]
pub enum ControlMsg {
    RoomUp { chan: Chan },
    RoomDown { chan: Chan, reason: String },
}

/// Why a session ended.
#[derive(Debug)]
pub enum SessionEnd {
    /// The gateway stopped for good (a bad token, a protocol problem).
    Fatal(Fatal),
    /// Closed on purpose (shutdown, a new token, another instance).
    Closed,
}

/// User name, display name, nickname, avatar.
type Names = (String, Option<String>, Option<String>, Option<String>);

struct Session {
    core: Arc<Core>,
    ctl: Arc<dyn FluxerCtl>,
    tx: mpsc::UnboundedSender<ControlMsg>,
    burst: Burst,
    machine: FollowMachine,
    session_id: Option<String>,
    grants: HashMap<(GuildId, ConnectionId), VoiceGrant>,
    stale_scan: bool,
    unstable_until: f64,
    gateway_ok: bool,
    presence: Option<String>,
    /// Names last recorded per person and community (only changes are recorded).
    seen: BTreeMap<(UserId, Option<GuildId>), Names>,
    communities: BTreeMap<GuildId, (String, Option<String>)>,
    blocked: BTreeSet<Chan>,
}

fn follow_cfg(core: &Core) -> FollowCfg {
    let eff = core.settings.current().effective(None, None);
    FollowCfg {
        settle_s: eff.join_settle.value.secs(),
        leave_grace_s: eff.leave_grace.value.secs(),
        ..FollowCfg::default()
    }
}

/// Runs one session until it ends.
pub async fn run(
    core: Arc<Core>,
    ctl: Arc<dyn FluxerCtl>,
    mut events: mpsc::UnboundedReceiver<GatewayEvent>,
) -> SessionEnd {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut s = Session {
        machine: FollowMachine::new(follow_cfg(&core)),
        core: core.clone(),
        ctl,
        tx,
        burst: Burst::default(),
        session_id: None,
        grants: HashMap::new(),
        stale_scan: false,
        unstable_until: 0.0,
        gateway_ok: false,
        presence: None,
        seen: BTreeMap::new(),
        // What the log already has is not recorded again.
        communities: core.guilds().known_communities.clone(),
        blocked: BTreeSet::new(),
    };
    let mut settings = core.settings.watch();
    let end = loop {
        let now = core.deps.clock.mono();
        let wake = s.machine.next_deadline().map_or(TICK, |d| {
            Duration::from_secs_f64((d - now).clamp(0.0, TICK.as_secs_f64()))
        });
        tokio::select! {
            ev = events.recv() => match ev {
                None => break SessionEnd::Closed,
                Some(GatewayEvent::Stopped(f)) => break SessionEnd::Fatal(f),
                Some(ev) => s.event(ev).await,
            },
            msg = rx.recv() => {
                let now = core.deps.clock.mono();
                match msg {
                    Some(ControlMsg::RoomUp { chan }) => {
                        let acts = s.machine.on_room_up(now, chan);
                        s.exec(acts);
                    }
                    Some(ControlMsg::RoomDown { chan, reason }) => {
                        core.set_room(chan, None);
                        let acts = s.machine.on_room_down(now, chan, &reason);
                        s.exec(acts);
                    }
                    None => {}
                }
            }
            _ = settings.changed() => {
                s.machine.cfg = follow_cfg(&core);
                core.mark_all();
            }
            () = tokio::time::sleep(wake) => {}
        }
        s.step();
    };
    // Leave every room (voice connections end with the session anyway).
    let acts = s.machine.shutdown(core.deps.clock.mono());
    s.exec(acts);
    for r in core.rooms() {
        let _ = r.tx.send(RoomCmd::Close);
        core.set_room(r.chan, None);
    }
    end
}

impl Session {
    async fn event(&mut self, ev: GatewayEvent) {
        let core = self.core.clone();
        let now = core.deps.clock.mono();
        if let Some(g) = core.update_guilds(|gs| gs.apply(&ev)) {
            core.mark_guild(g);
        }
        match &ev {
            GatewayEvent::Ready { bot, session, guilds } => {
                core.update_guilds(|gs| gs.bot = Some(bot.user.id));
                self.session_id = Some(session.clone());
                core.update_voice(|v| v.clear());
                self.burst.start(guilds.iter().copied(), now);
                self.stale_scan = true;
                self.gateway_ok = true;
                self.unstable_until = now + UNSTABLE_S;
                self.presence = None;
                let acts = self.machine.on_gateway_fresh(now);
                self.exec(acts);
                core.connection
                    .send_modify(|l| l.state = super::core::Connection::Ready);
                tracing::info!(bot = %bot.user.id, communities = guilds.len(), "logged in to Fluxer");
            }
            GatewayEvent::Resumed => {
                self.gateway_ok = true;
                self.unstable_until = now + UNSTABLE_S;
                core.connection
                    .send_modify(|l| l.state = super::core::Connection::Ready);
            }
            GatewayEvent::Down { code, resuming } => {
                self.gateway_ok = false;
                self.unstable_until = now + UNSTABLE_S;
                // The gateway's close code, if it sent one.
                let why = code.map_or_else(String::new, |c| c.to_string());
                core.connection
                    .send_modify(|l| l.state = super::core::Connection::Retrying(why));
                tracing::info!(
                    ?code,
                    resuming,
                    "the Fluxer gateway connection dropped; voice connections stay while it reconnects"
                );
            }
            GatewayEvent::GuildAvailable(g) => {
                core.update_voice(|v| v.guild_available(g.id, g.voice_states.iter().cloned()));
                self.burst.resolve(g.id);
                self.remember_community(g.id, &g.name, g.icon.clone()).await;
                let people: Vec<_> = g
                    .members
                    .iter()
                    .filter_map(|m| m.user.clone().map(|u| (u, m.nick.clone())))
                    .collect();
                for (u, nick) in people {
                    self.remember_person(Some(g.id), &u, nick).await;
                }
            }
            GatewayEvent::GuildUnavailable(g) => {
                core.update_voice(|v| v.guild_gone(*g, true));
                self.burst.resolve(*g);
            }
            GatewayEvent::GuildRemoved(g) => {
                core.update_voice(|v| v.guild_gone(*g, false));
                self.burst.resolve(*g);
            }
            GatewayEvent::GuildUpdated { guild, name, icon, .. } => {
                self.remember_community(*guild, name, icon.clone()).await
            }
            GatewayEvent::MemberUpdated { guild, member } => {
                if let Some(u) = member.user.clone() {
                    self.remember_person(Some(*guild), &u, member.nick.clone()).await;
                }
            }
            GatewayEvent::VoiceState { state, member } => {
                if let Some(u) = member.as_ref().and_then(|m| m.user.clone()) {
                    self.remember_person(Some(state.guild), &u, member.as_ref().and_then(|m| m.nick.clone()))
                        .await;
                }
                let (g, u) = (state.guild, state.user);
                core.update_voice(|v| v.update((**state).clone()));
                core.mark_person(g, u);
                let listening = core
                    .rooms()
                    .iter()
                    .any(|r| r.chan.guild == g && Some(r.chan.channel) == state.channel);
                let dot = match state.channel {
                    None => Dot::Away,
                    Some(_) if listening => Dot::Listening,
                    Some(_) => Dot::InCall,
                };
                core.live.dot(g, u, dot);
            }
            GatewayEvent::VoiceServer(grant) => {
                let VoiceGrant::LiveKit {
                    guild,
                    channel,
                    connection,
                    e2ee_key,
                    ..
                } = grant.as_ref()
                else {
                    tracing::warn!("a voice grant this build cannot use");
                    return;
                };
                let info = GrantInfo {
                    chan: Chan {
                        guild: *guild,
                        channel: *channel,
                    },
                    connection: connection.clone(),
                    has_e2ee_key: e2ee_key.is_some(),
                };
                self.grants.insert((*guild, connection.clone()), (**grant).clone());
                let acts = self.machine.on_grant(now, &info);
                self.exec(acts);
            }
            GatewayEvent::Message(m) => {
                let core2 = core.clone();
                let m = m.clone();
                tokio::spawn(async move { super::commands::on_message(&core2, &m).await });
            }
            _ => {}
        }
    }

    async fn remember_community(&mut self, guild: GuildId, name: &str, icon: Option<String>) {
        let v = (name.to_owned(), icon.clone());
        if self.communities.get(&guild) != Some(&v) {
            self.communities.insert(guild, v.clone());
            self.core.update_guilds(|gs| {
                gs.known_communities.insert(guild, v);
            });
            self.core
                .record(vec![Event::CommunitySeen(CommunitySeen {
                    guild,
                    name: name.to_owned(),
                    icon,
                })])
                .await;
        }
    }

    async fn remember_person(&mut self, guild: Option<GuildId>, u: &pb_fluxer_api::User, nick: Option<String>) {
        let v = (
            u.username.clone(),
            u.global_name.clone(),
            nick.clone(),
            u.avatar.clone(),
        );
        let key = (u.id, guild);
        if self.seen.get(&key) != Some(&v) {
            self.seen.insert(key, v);
            let ev = PersonSeen {
                user: u.id,
                guild,
                username: u.username.clone(),
                display_name: u.global_name.clone(),
                nick,
                avatar: u.avatar.clone(),
            };
            self.core.record(vec![Event::PersonSeen(ev)]).await;
        }
    }

    fn step(&mut self) {
        let core = self.core.clone();
        let now = core.deps.clock.mono();
        let burst_done = self.burst.done(now, BURST_CAP_S);
        self.machine.enabled = burst_done && self.gateway_ok;
        self.machine.gateway_ok = self.gateway_ok;
        let tree = core.settings.current();
        let bot = core.guilds().bot;
        let (desired, blocked, own, stale, unavailable) = {
            let world = core.voice();
            let (desired, blocked) = desired_channels(&world, &|g| tree.tracked_for(g), bot, &|g| {
                tree.effective(Some(g), None).allow_e2ee_downgrade.value
            });
            let own: BTreeMap<(GuildId, ConnectionId), ChannelId> = bot
                .map(|b| world.of_user(b).map(|v| (v.key(), v.channel)).collect())
                .unwrap_or_default();
            let stale = if self.stale_scan && burst_done {
                self.stale_scan = false;
                bot.map(|b| stale_own_states(&world, b, self.session_id.as_deref(), &self.machine.known_connections()))
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            (desired, blocked, own, stale, world.unavailable.clone())
        };
        for chan in &blocked {
            if self.blocked.insert(*chan) {
                tracing::warn!(
                    ?chan,
                    "not joining: the channel is end-to-end encrypted and a bot joining would switch that off for everyone (allow it with the setting allow_e2ee_downgrade)"
                );
            }
        }
        self.blocked.retain(|c| blocked.contains(c));
        let mut acts = self
            .machine
            .reconcile(now, &desired, &own, &stale, now >= self.unstable_until, &unavailable);
        acts.extend(self.machine.on_tick(now));
        self.exec(acts);
        let conns: std::collections::BTreeMap<Chan, pb_live_proto::BotJoin> =
            self.machine.conns().map(|c| (c.chan, bot_join(c.state))).collect();
        if let Ok(mut w) = core.conns.write()
            && *w != conns
        {
            for chan in w.keys().chain(conns.keys()) {
                core.mark_guild(chan.guild);
            }
            *w = conns;
        }
        if burst_done && self.gateway_ok {
            let text = super::commands::presence_text(&core);
            if self.presence.as_ref() != Some(&text) {
                self.ctl.presence(Some(text.clone()));
                self.presence = Some(text);
            }
        }
    }

    fn exec(&mut self, acts: Vec<Action>) {
        for a in acts {
            match a {
                Action::VoiceState {
                    guild,
                    channel,
                    connection,
                } => {
                    let op = match (channel, connection) {
                        (Some(channel), None) => VoiceStateOp::Join { guild, channel },
                        (Some(channel), Some(connection)) => VoiceStateOp::Update {
                            guild,
                            channel,
                            connection,
                        },
                        (None, Some(connection)) => VoiceStateOp::Leave { guild, connection },
                        (None, None) => continue,
                    };
                    let ctl = self.ctl.clone();
                    // A leave is retried while the gateway reconnects (it would be lost); joins and confirmations
                    // are covered by the follow machine's timeouts.
                    let attempts = if matches!(op, VoiceStateOp::Leave { .. }) { 6 } else { 1 };
                    tokio::spawn(async move {
                        for i in 0..attempts {
                            match ctl.voice_state(op.clone()).await {
                                Ok(()) => return,
                                Err(e) if i + 1 == attempts => {
                                    tracing::debug!(error = %e, ?op, "a voice-state update was not sent")
                                }
                                Err(_) => tokio::time::sleep(Duration::from_secs(2)).await,
                            }
                        }
                    });
                }
                Action::Connect { chan, connection } => {
                    let Some(grant) = self.grants.get(&(chan.guild, connection.clone())).cloned() else {
                        tracing::warn!(?chan, "no voice grant for this connection");
                        let _ = self.tx.send(ControlMsg::RoomDown {
                            chan,
                            reason: "no voice grant".into(),
                        });
                        continue;
                    };
                    let timeout = Duration::from_secs_f64(self.machine.cfg.connect_timeout_s);
                    let handle = super::room::spawn(self.core.clone(), chan, grant, self.tx.clone(), timeout);
                    self.core.set_room(chan, Some(handle));
                }
                Action::Close { chan } => {
                    if let Some(r) = self.core.room(chan) {
                        let _ = r.tx.send(RoomCmd::Close);
                    }
                    self.core.set_room(chan, None);
                }
                Action::Notice { kind, text, chan } => {
                    use pb_policy::NoticeKind as K;
                    match kind {
                        K::StaleLeave | K::Involuntary | K::LateGrant => tracing::info!(?chan, "{text}"),
                        _ => tracing::warn!(?chan, "{text}"),
                    }
                }
            }
        }
    }
}

/// The follow machine's state as the pages show it.
fn bot_join(s: pb_policy::ConnState) -> pb_live_proto::BotJoin {
    use pb_live_proto::BotJoin;
    use pb_policy::ConnState;
    match s {
        ConnState::Settling => BotJoin::Waiting,
        ConnState::Joining | ConnState::Connecting | ConnState::Confirming => BotJoin::Joining,
        ConnState::Active => BotJoin::Connected,
        ConnState::Leaving => BotJoin::Leaving,
        ConnState::Backoff => BotJoin::Retrying,
    }
}
