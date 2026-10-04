//! What the bot knows about each community: name, owner, roles, channels, and the people it has seen (names), kept
//! current from the gateway so permission checks and names are always up to date.

use std::collections::{BTreeMap, BTreeSet};

use pb_domain::{ChannelId, GuildId, RoleId, UserId};
use pb_fluxer_api::{Channel, GatewayEvent, Guild, Member, Role, User, perms};

/// A person's names in a community.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub user: UserId,
    pub username: String,
    pub display_name: Option<String>,
    pub nick: Option<String>,
    pub avatar: Option<String>,
    pub roles: Vec<RoleId>,
    pub bot: bool,
}

impl Person {
    /// Nickname, else display name, else user name.
    pub fn shown(&self) -> String {
        self.nick
            .clone()
            .or_else(|| self.display_name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                if self.username.is_empty() {
                    self.user.to_string()
                } else {
                    self.username.clone()
                }
            })
    }
}

#[derive(Debug, Clone, Default)]
pub struct GuildInfo {
    pub name: String,
    pub icon: Option<String>,
    pub owner: Option<UserId>,
    pub available: bool,
    pub roles: BTreeMap<RoleId, Role>,
    pub channels: BTreeMap<ChannelId, Channel>,
    pub people: BTreeMap<UserId, Person>,
}

/// Every community.
#[derive(Debug, Clone, Default)]
pub struct Guilds {
    pub bot: Option<UserId>,
    pub guilds: BTreeMap<GuildId, GuildInfo>,
    /// The names last seen for people the gateway has not told about (yet): from the log at start, and from every
    /// member seen since. Kept across reconnects.
    pub known: BTreeMap<(GuildId, UserId), Person>,
    /// The name and icon last seen of every community the bot was in (its history names left communities too).
    pub known_communities: BTreeMap<GuildId, (String, Option<String>)>,
}

fn merge_user(p: &mut Person, u: &User) {
    if !u.username.is_empty() {
        p.username.clone_from(&u.username);
    }
    if u.global_name.is_some() {
        p.display_name.clone_from(&u.global_name);
    }
    if u.avatar.is_some() {
        p.avatar.clone_from(&u.avatar);
    }
    p.bot = u.bot;
}

impl GuildInfo {
    fn put_member(&mut self, m: &Member) {
        let p = self.people.entry(m.id).or_insert_with(|| Person {
            user: m.id,
            username: String::new(),
            display_name: None,
            nick: None,
            avatar: None,
            roles: Vec::new(),
            bot: false,
        });
        if let Some(u) = &m.user {
            merge_user(p, u);
        }
        p.nick.clone_from(&m.nick);
        p.roles.clone_from(&m.roles);
    }
}

impl Guilds {
    pub fn clear(&mut self) {
        self.guilds.clear();
    }

    pub fn get(&self, g: GuildId) -> Option<&GuildInfo> {
        self.guilds.get(&g)
    }

    pub fn available(&self) -> BTreeSet<GuildId> {
        self.guilds
            .iter()
            .filter(|(_, g)| g.available)
            .map(|(id, _)| *id)
            .collect()
    }

    fn put_guild(&mut self, g: &Guild) {
        let mut info = GuildInfo {
            name: g.name.clone(),
            icon: g.icon.clone(),
            owner: g.owner,
            available: true,
            roles: g.roles.iter().map(|r| (r.id, r.clone())).collect(),
            channels: g.channels.iter().map(|c| (c.id, c.clone())).collect(),
            people: self.guilds.get(&g.id).map(|old| old.people.clone()).unwrap_or_default(),
        };
        for m in &g.members {
            info.put_member(m);
        }
        self.guilds.insert(g.id, info);
    }

    /// Applies a gateway event; returns the community it changed.
    pub fn apply(&mut self, ev: &GatewayEvent) -> Option<GuildId> {
        match ev {
            GatewayEvent::Ready { .. } => {
                self.guilds.clear();
                None
            }
            GatewayEvent::GuildAvailable(g) => {
                self.put_guild(g);
                Some(g.id)
            }
            GatewayEvent::GuildUnavailable(g) => {
                self.guilds.entry(*g).or_default().available = false;
                Some(*g)
            }
            GatewayEvent::GuildRemoved(g) => {
                self.guilds.remove(g);
                Some(*g)
            }
            GatewayEvent::GuildUpdated {
                guild,
                name,
                icon,
                owner,
            } => {
                let g = self.guilds.entry(*guild).or_default();
                g.name.clone_from(name);
                g.icon.clone_from(icon);
                if owner.is_some() {
                    g.owner = *owner;
                }
                Some(*guild)
            }
            GatewayEvent::RolesChanged { guild, upsert, removed } => {
                let g = self.guilds.entry(*guild).or_default();
                for r in upsert {
                    g.roles.insert(r.id, r.clone());
                }
                for r in removed {
                    g.roles.remove(r);
                }
                Some(*guild)
            }
            GatewayEvent::ChannelsChanged { guild, upsert, removed } => {
                let g = self.guilds.entry(*guild).or_default();
                for c in upsert {
                    g.channels.insert(c.id, c.clone());
                }
                for c in removed {
                    g.channels.remove(c);
                }
                Some(*guild)
            }
            GatewayEvent::MemberUpdated { guild, member } => {
                self.guilds.entry(*guild).or_default().put_member(member);
                Some(*guild)
            }
            GatewayEvent::MemberRemoved { guild, user } => {
                if let Some(g) = self.guilds.get_mut(guild) {
                    g.people.remove(user);
                }
                Some(*guild)
            }
            GatewayEvent::VoiceState { state, member } => {
                if let Some(m) = member {
                    self.guilds.entry(state.guild).or_default().put_member(m);
                }
                None
            }
            GatewayEvent::Message(m) => {
                let g = m.guild?;
                let info = self.guilds.entry(g).or_default();
                let p = info.people.entry(m.author.id).or_insert_with(|| Person {
                    user: m.author.id,
                    username: String::new(),
                    display_name: None,
                    nick: None,
                    avatar: None,
                    roles: Vec::new(),
                    bot: false,
                });
                merge_user(p, &m.author);
                if m.author_nick.is_some() {
                    p.nick.clone_from(&m.author_nick);
                }
                p.roles.clone_from(&m.author_roles);
                None
            }
            _ => None,
        }
    }

    /// A person's name in a community (the id when never seen).
    pub fn name(&self, g: GuildId, u: UserId) -> String {
        self.person(g, u).map_or_else(|| u.to_string(), Person::shown)
    }

    /// What the gateway says about a person, else what was last seen.
    pub fn person(&self, g: GuildId, u: UserId) -> Option<&Person> {
        self.guilds
            .get(&g)
            .and_then(|i| i.people.get(&u))
            .or_else(|| self.known.get(&(g, u)))
    }

    /// Notes a person's names (from the log or a lookup).
    pub fn remember(&mut self, g: GuildId, p: Person) {
        self.known.insert((g, p.user), p);
    }

    pub fn channel_name(&self, g: GuildId, c: ChannelId) -> String {
        self.guilds
            .get(&g)
            .and_then(|i| i.channels.get(&c))
            .map_or_else(|| c.to_string(), |ch| ch.name.clone())
    }

    pub fn guild_name(&self, g: GuildId) -> String {
        self.guilds
            .get(&g)
            .map(|i| i.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| self.known_communities.get(&g).map(|(n, _)| n.clone()))
            .unwrap_or_else(|| g.to_string())
    }

    /// `user`'s permissions in a community (and channel), given their roles.
    pub fn permissions(&self, g: GuildId, user: UserId, roles: &[RoleId], channel: Option<ChannelId>) -> u64 {
        let Some(info) = self.guilds.get(&g) else { return 0 };
        let roles_all: Vec<Role> = info.roles.values().cloned().collect();
        let ch = channel.and_then(|c| info.channels.get(&c));
        perms::compute(g, info.owner, &roles_all, user, roles, ch)
    }

    /// The bot's permissions in a channel.
    pub fn bot_permissions(&self, g: GuildId, channel: Option<ChannelId>) -> u64 {
        let Some(bot) = self.bot else { return 0 };
        let roles = self.person(g, bot).map(|p| p.roles.clone()).unwrap_or_default();
        self.permissions(g, bot, &roles, channel)
    }
}
