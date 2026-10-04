//! The live views (hub cells) the engine keeps: the sidebar's communities and people, the wall's tiles, each
//! community's page and each person's page. Communities and people are built when someone opens their page
//! ([`Cells`] is the hub's cell source) and rebuilt a few times a second while something changed.

use std::collections::{BTreeSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use pb_domain::{Audience, GuildId, UserId};
use pb_live::CellSource;
use pb_live_proto::{
    Call, ChannelRef, Connection, Counts, Dot, GuildDelta, GuildState, Participant, PersonDelta, PersonState, Presence,
    SentenceCard, SidebarCommunity, SidebarDelta, SidebarPerson, Stamps, Topic, TopicState, TrackedPerson, Tracking,
    WallDelta, Who,
};
use pb_policy::{Chan, e2ee_active};
use pb_store_api::{SentenceFilter, SentenceKind};

use super::core::Core;
use super::live::{person_topic, summary};
use super::moderation::decision_view;

/// What changed since the last rebuild.
#[derive(Debug, Default)]
pub struct Dirty {
    pub guilds: BTreeSet<GuildId>,
    pub people: BTreeSet<(GuildId, UserId)>,
    pub all: bool,
}

pub(crate) fn who(core: &Core, g: GuildId, u: UserId) -> Who {
    let gs = core.guilds();
    // The picture's address on the instance's media server (the gateway sends only its hash).
    let avatar = core.avatar_url(u, gs.person(g, u).and_then(|p| p.avatar.as_deref()));
    Who {
        user: u,
        name: gs.name(g, u),
        avatar,
    }
}

/// A person's dot.
pub fn dot(core: &Core, g: GuildId, u: UserId) -> Dot {
    let channel = core.voice().of_user(u).find(|v| v.guild == g).map(|v| v.channel);
    match channel {
        None => Dot::Away,
        Some(_) if core.is_listening(g, u) => Dot::Listening,
        Some(_) => Dot::InCall,
    }
}

/// A community's sidebar entry.
pub fn sidebar_community(core: &Core, g: GuildId) -> SidebarCommunity {
    let tree = core.settings.current();
    let (icon, available) = core
        .guilds()
        .get(g)
        .map_or((None, false), |i| (i.icon.clone(), i.available));
    let mut people: Vec<SidebarPerson> = tree
        .listed_for(g)
        .into_iter()
        .map(|u| SidebarPerson {
            who: who(core, g, u),
            dot: dot(core, g, u),
            paused: tree.effective(Some(g), Some(u)).paused.value,
        })
        .collect();
    people.sort_by_key(|a| a.who.name.to_lowercase());
    SidebarCommunity {
        id: g,
        name: core.guilds().guild_name(g),
        icon,
        available,
        paused: tree.effective(Some(g), None).paused.value,
        people,
    }
}

/// The communities in the sidebar: the ones the bot is in and the ones with settings.
pub(crate) fn communities(core: &Core) -> BTreeSet<GuildId> {
    let mut out: BTreeSet<GuildId> = core.guilds().guilds.keys().copied().collect();
    out.extend(core.settings.current().servers.keys().copied());
    out
}

/// A community's page.
pub fn guild_state(core: &Core, g: GuildId) -> GuildState {
    let tree = core.settings.current();
    let eff = tree.effective(Some(g), None);
    let listed = tree.listed_for(g);
    let active = tree.tracked_for(g);
    let everywhere = eff.tracked_everywhere.value.clone();
    let bot = core.guilds().bot;
    let gs = core.guilds();
    let info = gs.get(g).cloned();
    drop(gs);
    let world = core.voice().clone();
    let mut by_channel: std::collections::BTreeMap<pb_domain::ChannelId, Vec<&pb_policy::VState>> = Default::default();
    for v in world.states().filter(|v| v.guild == g) {
        by_channel.entry(v.channel).or_default().push(v);
    }
    let rooms: BTreeSet<Chan> = core.rooms().into_iter().map(|r| r.chan).collect();
    let speaking = core.speaking.lock().map(|s| s.clone()).unwrap_or_default();
    let mut calls = Vec::new();
    for (channel, states) in by_channel {
        let chan = Chan { guild: g, channel };
        let mut participants: Vec<Participant> = states
            .iter()
            .map(|v| Participant {
                who: who(core, g, v.user),
                bot: Some(v.user) == bot,
                tracked: listed.contains(&v.user),
                active: active.contains(&v.user),
                muted: v.mute || v.self_mute,
                deaf: v.deaf,
                listening: core.is_listening(g, v.user),
            })
            .collect();
        participants.sort_by_key(|p| (!p.bot, !p.tracked, p.who.name.to_lowercase()));
        calls.push(Call {
            channel: ChannelRef {
                id: channel,
                name: core.guilds().channel_name(g, channel),
            },
            participants,
            bot_in: rooms.contains(&chan),
            can_speak: speaking.contains(&chan),
            encrypted: e2ee_active(&world, chan, bot),
            audience: Audience::from(eff.audience.value),
        });
    }
    let mut tracked: Vec<TrackedPerson> = listed
        .iter()
        .map(|u| TrackedPerson {
            who: who(core, g, *u),
            active: active.contains(u),
            paused: tree.effective(Some(g), Some(*u)).paused.value,
            everywhere: everywhere.contains(u),
            channel: world.of_user(*u).find(|v| v.guild == g).map(|v| v.channel),
        })
        .collect();
    tracked.sort_by_key(|a| a.who.name.to_lowercase());
    let connections = core
        .conns
        .read()
        .map(|c| {
            c.iter()
                .filter(|(chan, _)| chan.guild == g)
                .map(|(chan, st)| Connection {
                    channel: chan.channel,
                    state: *st,
                })
                .collect()
        })
        .unwrap_or_default();
    GuildState {
        id: g,
        name: core.guilds().guild_name(g),
        icon: info.as_ref().and_then(|i| i.icon.clone()),
        available: info.as_ref().is_some_and(|i| i.available),
        allowed: tree.guild_allowed(g),
        paused: eff.paused.value,
        calls,
        tracked,
        connections,
        violations: VecDeque::new(),
    }
}

fn presence(core: &Core, g: GuildId, u: UserId) -> Presence {
    let v = core.voice().of_user(u).find(|v| v.guild == g).cloned();
    let rooms: BTreeSet<Chan> = core.rooms().into_iter().map(|r| r.chan).collect();
    match v {
        None => Presence::default(),
        Some(v) => Presence {
            channel: Some(ChannelRef {
                id: v.channel,
                name: core.guilds().channel_name(g, v.channel),
            }),
            muted: v.mute || v.self_mute,
            deaf: v.deaf,
            listening: core.is_listening(g, u),
            bot_in_call: rooms.contains(&Chan {
                guild: g,
                channel: v.channel,
            }),
        },
    }
}

pub(crate) fn tracking(core: &Core, g: GuildId, u: UserId) -> Tracking {
    let tree = core.settings.current();
    Tracking {
        listed: tree.listed_for(g).contains(&u),
        active: tree.tracked_for(g).contains(&u),
        everywhere: tree.effective(None, None).tracked_everywhere.value.contains(&u),
    }
}

/// A person's page (without their recent sentences, which come from the index).
pub fn person_state(core: &Core, g: GuildId, u: UserId) -> PersonState {
    let eff = core.settings.current().effective(Some(g), Some(u));
    PersonState {
        guild: g,
        who: who(core, g, u),
        presence: presence(core, g, u),
        tracking: tracking(core, g, u),
        counts: Counts {
            jar: core.jar(g, u),
            window_ms: eff.violation_window.value.value().map(|d| d.get().millis()),
            ..Counts::default()
        },
        summary: summary(&eff),
        levels: Default::default(),
        sentences: Vec::new(),
        activity: Default::default(),
    }
}

/// Builds cells on demand for the hub.
#[derive(Clone)]
pub struct Cells {
    pub(crate) core: Arc<Core>,
}

impl std::fmt::Debug for Cells {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Cells")
    }
}

impl CellSource for Cells {
    /// Builds a community's or person's view the first time someone watches it (later watchers share it).
    fn ensure(&self, topic: &Topic) -> bool {
        let core = &self.core;
        match topic {
            Topic::Guild { guild } => {
                if !communities(core).contains(guild) {
                    return false;
                }
                if !core.live.hub.has(topic) {
                    core.live
                        .hub
                        .set(topic.clone(), TopicState::Guild(Box::new(guild_state(core, *guild))));
                }
                true
            }
            Topic::Person { guild, user } => {
                // Only people of a known community the bot has seen there or tracks there.
                let known = core.guilds().person(*guild, *user).is_some()
                    || core.settings.current().listed_for(*guild).contains(user);
                if !communities(core).contains(guild) || !known {
                    return false;
                }
                if !core.live.hub.has(topic) {
                    core.live.hub.set(
                        topic.clone(),
                        TopicState::Person(Box::new(person_state(core, *guild, *user))),
                    );
                    let (core2, g, u) = (core.clone(), *guild, *user);
                    tokio::spawn(async move { fill_person(&core2, g, u).await });
                }
                true
            }
            _ => core.live.hub.has(topic),
        }
    }
}

/// What a person's view needs from elsewhere: the counts (from the moderation actor) and the latest sentences (from
/// the log's index).
pub(crate) async fn fill_person(core: &Core, g: GuildId, u: UserId) {
    let (tx, rx) = tokio::sync::oneshot::channel();
    if core
        .moderation
        .send(super::moderation::ModMsg::Counts(g, u, tx))
        .is_ok()
        && let Ok(counts) = rx.await
    {
        core.live.person(g, u, PersonDelta::Counts { counts });
    }
    load_recent(core, g, u).await;
}

/// The person's most recent sentences, as conveyor cards (all finished).
async fn load_recent(core: &Core, g: GuildId, u: UserId) {
    let f = SentenceFilter {
        guilds: Some(vec![g]),
        user: Some(u),
        kind: SentenceKind::All,
        ..SentenceFilter::default()
    };
    let Ok(page) = core
        .deps
        .index
        .sentences(&f, None, u32::try_from(pb_live_proto::LIVE_SENTENCES).unwrap_or(50))
        .await
    else {
        return;
    };
    for (i, row) in page.items.iter().rev().enumerate() {
        let r = &row.record;
        let opened = r.started.as_millisecond();
        let end = opened + i64::from(r.dur_ms);
        let card = SentenceCard {
            id: r.id,
            no: u32::try_from(i + 1).unwrap_or(0),
            stamps: Stamps {
                opened,
                cut: Some(end),
                queued: Some(end),
                scoring: Some(end),
                scored: Some(end),
                decided: Some(end),
                ..Stamps::default()
            },
            dur_ms: Some(r.dur_ms),
            level_db: r.level_db,
            cut: None,
            dropped: None,
            error: None,
            verdict: Some(pb_live_proto::VerdictView {
                scores: r.scores,
                thresholds: r.thresholds.clone(),
                flagged: r.flagged.clone(),
                language: r.language,
                infer_ms: r.infer_ms.unwrap_or(0),
                cut_to_verdict_ms: r.cut_to_verdict_ms.unwrap_or(0),
            }),
            decision: Some(decision_view(&r.decision)),
        };
        core.live.person(g, u, PersonDelta::Sentence { card: Box::new(card) });
    }
}

/// Rebuilds what changed, four times a second.
pub async fn refresh(core: Arc<Core>) {
    let mut sidebar_known: BTreeSet<GuildId> = BTreeSet::new();
    let mut wall_tiles: BTreeSet<(GuildId, UserId)> = BTreeSet::new();
    let mut swept = std::time::Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        // Person views nobody watches any more go (they are built again when someone opens one).
        if swept.elapsed() > Duration::from_secs(60) {
            swept = std::time::Instant::now();
            core.live.hub.drop_unwatched(|t| !matches!(t, Topic::Person { .. }));
        }
        let d = core
            .dirty
            .lock()
            .map(|mut d| std::mem::take(&mut *d))
            .unwrap_or_default();
        let guilds: BTreeSet<GuildId> = if d.all { communities(&core) } else { d.guilds.clone() };
        // The sidebar: communities appear, change and go.
        let now = communities(&core);
        for g in guilds.iter().chain(now.difference(&sidebar_known)) {
            if now.contains(g) {
                core.live.sidebar(SidebarDelta::Community {
                    community: sidebar_community(&core, *g),
                });
            }
        }
        for g in sidebar_known.difference(&now) {
            core.live.sidebar(SidebarDelta::CommunityGone { guild: *g });
        }
        sidebar_known = now;
        // Community pages that are open.
        for g in &guilds {
            let t = Topic::Guild { guild: *g };
            if core.live.hub.has(&t) {
                core.live.hub.publish(
                    &t,
                    pb_live_proto::TopicDelta::Guild(GuildDelta::View {
                        view: Box::new(guild_state(&core, *g)),
                    }),
                );
            }
        }
        // Person pages that are open.
        let people: BTreeSet<(GuildId, UserId)> = if d.all {
            core.live.hub_people()
        } else {
            d.people.clone()
        };
        for (g, u) in people {
            if core.live.hub.has(&person_topic(g, u)) {
                core.live.person(
                    g,
                    u,
                    PersonDelta::Presence {
                        presence: presence(&core, g, u),
                    },
                );
                core.live.person(
                    g,
                    u,
                    PersonDelta::Tracking {
                        tracking: tracking(&core, g, u),
                    },
                );
                core.live.person(g, u, PersonDelta::Who { who: who(&core, g, u) });
                let eff = core.settings.current().effective(Some(g), Some(u));
                core.live.person(g, u, PersonDelta::Summary { summary: summary(&eff) });
            }
        }
        // The wall: one tile per person the bot listens to.
        let listening: BTreeSet<(GuildId, UserId)> = core.listening.lock().map(|l| l.clone()).unwrap_or_default();
        for (g, u) in listening.difference(&wall_tiles) {
            let channel = core.voice().of_user(*u).find(|v| v.guild == *g).map(|v| v.channel);
            let Some(channel) = channel else { continue };
            let tile = pb_live_proto::Tile {
                guild: *g,
                community: core.guilds().guild_name(*g),
                who: who(&core, *g, *u),
                channel: ChannelRef {
                    id: channel,
                    name: core.guilds().channel_name(*g, channel),
                },
                levels: Default::default(),
                latest: None,
                lag_ms: None,
            };
            core.live.wall(WallDelta::Tile { tile: Box::new(tile) });
        }
        for (g, u) in wall_tiles.difference(&listening) {
            core.live.wall(WallDelta::TileGone { guild: *g, user: *u });
        }
        wall_tiles = listening;
    }
}
