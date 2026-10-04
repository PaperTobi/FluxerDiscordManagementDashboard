//! Publishing what happens to the live hub (the web UI's topics). Cheap when nobody watches: high-rate updates (levels)
//! are only computed for watched topics.

use pb_domain::{Audience, GuildId, UserId};
use pb_live::Hub;
use pb_live_proto::{
    Counts, Dot, LevelRun, PersonDelta, PersonState, PersonSummary, Presence, SentenceCard, SidebarDelta, SidebarState,
    SystemState, Topic, TopicDelta, TopicState, Tracking, ViolationItem, WallDelta, WallState, Who,
};

/// The engine's view of the hub.
#[derive(Debug, Clone)]
pub struct Live {
    pub hub: Hub,
}

pub fn person_topic(guild: GuildId, user: UserId) -> Topic {
    Topic::Person { guild, user }
}

impl Live {
    pub fn new(hub: Hub) -> Live {
        if !hub.has(&Topic::Sidebar) {
            hub.set(Topic::Sidebar, TopicState::Sidebar(SidebarState::default()));
        }
        if !hub.has(&Topic::Wall) {
            hub.set(Topic::Wall, TopicState::Wall(WallState::default()));
        }
        Live { hub }
    }

    pub fn sidebar(&self, d: SidebarDelta) {
        self.hub.publish(&Topic::Sidebar, TopicDelta::Sidebar(d));
    }

    pub fn dot(&self, guild: GuildId, user: UserId, dot: Dot) {
        self.sidebar(SidebarDelta::Dot { guild, user, dot });
    }

    pub fn wall(&self, d: WallDelta) {
        self.hub.publish(&Topic::Wall, TopicDelta::Wall(d));
    }

    pub fn system(&self, s: SystemState) {
        let t = Topic::System;
        if self.hub.has(&t) {
            self.hub.publish(
                &t,
                TopicDelta::System(pb_live_proto::SystemDelta::Status { status: Box::new(s) }),
            );
        } else {
            self.hub.set(t, TopicState::System(Box::new(s)));
        }
    }

    /// Makes sure a person has a cell (created with what is known now).
    pub fn ensure_person(&self, guild: GuildId, who: Who, summary: PersonSummary) {
        let t = person_topic(guild, who.user);
        if !self.hub.has(&t) {
            let state = PersonState {
                guild,
                who,
                presence: Presence::default(),
                tracking: Tracking::default(),
                counts: Counts::default(),
                summary,
                levels: Default::default(),
                sentences: Vec::new(),
                activity: Default::default(),
            };
            self.hub.set(t, TopicState::Person(Box::new(state)));
        }
    }

    pub fn person(&self, guild: GuildId, user: UserId, d: PersonDelta) {
        self.hub.publish(&person_topic(guild, user), TopicDelta::Person(d));
    }

    /// People whose pages have cells.
    pub fn hub_people(&self) -> std::collections::BTreeSet<(GuildId, UserId)> {
        self.hub
            .topics()
            .into_iter()
            .filter_map(|t| match t {
                Topic::Person { guild, user } => Some((guild, user)),
                _ => None,
            })
            .collect()
    }

    pub fn watched(&self, guild: GuildId, user: UserId) -> bool {
        self.hub.watched(&person_topic(guild, user)) || self.hub.watched(&Topic::Wall)
    }

    /// A sentence card changed: the person's conveyor and the wall tile.
    pub fn sentence(&self, guild: GuildId, user: UserId, card: &SentenceCard) {
        self.person(
            guild,
            user,
            PersonDelta::Sentence {
                card: Box::new(card.clone()),
            },
        );
        self.wall(WallDelta::Sentence {
            guild,
            user,
            card: Box::new(card.clone()),
        });
    }

    pub fn levels(&self, guild: GuildId, user: UserId, run: LevelRun) {
        if self.hub.watched(&person_topic(guild, user)) {
            self.person(guild, user, PersonDelta::Levels { run: run.clone() });
        }
        if self.hub.watched(&Topic::Wall) {
            self.wall(WallDelta::Levels { guild, user, run });
        }
    }

    pub fn violation(&self, v: ViolationItem) {
        let g = Topic::Guild { guild: v.guild };
        if self.hub.has(&g) {
            self.hub.publish(
                &g,
                TopicDelta::Guild(pb_live_proto::GuildDelta::Violation { violation: v.clone() }),
            );
        }
        self.wall(WallDelta::Violation { violation: v });
    }
}

/// The settings that shape a person's live view.
pub fn summary(eff: &pb_settings::Effective) -> PersonSummary {
    PersonSummary {
        observe_only: eff.observe_only.value,
        audience: Audience::from(eff.audience.value),
        strikes: eff.strikes.value.get(),
        thresholds: eff
            .enabled_labels()
            .into_iter()
            .map(|l| (l, eff.threshold_for(l) as f32))
            .collect(),
    }
}
