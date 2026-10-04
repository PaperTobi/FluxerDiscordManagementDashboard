//! Who is in which voice channel, from the guild snapshots and voice-state updates.

use std::collections::{BTreeMap, BTreeSet};

use pb_domain::{ChannelId, ConnectionId, GuildId, UserId, VoiceState};

/// A voice connection that is in a channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VState {
    pub guild: GuildId,
    pub channel: ChannelId,
    pub user: UserId,
    pub connection: ConnectionId,
    pub session: Option<String>,
    pub e2ee_capable: bool,
    /// Muted by themselves (a self-deafened client is muted too).
    pub self_mute: bool,
    /// Muted by the community (or suppressed).
    pub mute: bool,
    /// Hears nothing (deafened by themselves or the community).
    pub deaf: bool,
    pub version: u64,
    /// Bumped when the connection appears or changes channel (recency).
    pub seq: u64,
}

impl VState {
    pub fn key(&self) -> (GuildId, ConnectionId) {
        (self.guild, self.connection.clone())
    }
}

/// The voice world.
#[derive(Debug, Clone, Default)]
pub struct VoiceWorld {
    pub available: BTreeSet<GuildId>,
    pub unavailable: BTreeSet<GuildId>,
    states: BTreeMap<(GuildId, ConnectionId), VState>,
    seq: u64,
}

impl VoiceWorld {
    pub fn clear(&mut self) {
        self.available.clear();
        self.unavailable.clear();
        self.states.clear();
    }

    /// A community's full snapshot (GUILD_CREATE): replaces its voice states.
    pub fn guild_available(&mut self, guild: GuildId, voice_states: impl IntoIterator<Item = VoiceState>) {
        self.drop_guild(guild);
        self.unavailable.remove(&guild);
        self.available.insert(guild);
        for vs in voice_states {
            if vs.guild == guild {
                self.put(vs);
            }
        }
    }

    /// The community is unavailable (an outage) or the bot left it.
    pub fn guild_gone(&mut self, guild: GuildId, unavailable: bool) {
        self.drop_guild(guild);
        self.available.remove(&guild);
        if unavailable {
            self.unavailable.insert(guild);
        } else {
            self.unavailable.remove(&guild);
        }
    }

    /// One update. Returns the new state, or the removed one when somebody left (`None` = ignored).
    pub fn update(&mut self, vs: VoiceState) -> Option<(VState, bool)> {
        let key = (vs.guild, vs.connection.clone());
        if let Some(old) = self.states.get(&key)
            && vs.version != 0
            && vs.version < old.version
        {
            return None;
        }
        if vs.channel.is_none() {
            return self.states.remove(&key).map(|old| (old, false));
        }
        self.put(vs).map(|s| (s, true))
    }

    fn put(&mut self, vs: VoiceState) -> Option<VState> {
        let channel = vs.channel?;
        let key = (vs.guild, vs.connection.clone());
        let seq = match self.states.get(&key) {
            Some(old) if old.channel == channel => old.seq,
            _ => {
                self.seq += 1;
                self.seq
            }
        };
        let s = VState {
            guild: vs.guild,
            channel,
            user: vs.user,
            connection: vs.connection,
            session: vs.session,
            e2ee_capable: vs.e2ee_capable,
            self_mute: vs.self_mute || vs.self_deaf,
            mute: vs.mute || vs.suppress,
            deaf: vs.deaf || vs.self_deaf,
            version: vs.version,
            seq,
        };
        self.states.insert(key, s.clone());
        Some(s)
    }

    fn drop_guild(&mut self, guild: GuildId) {
        self.states.retain(|k, _| k.0 != guild);
    }

    pub fn states(&self) -> impl Iterator<Item = &VState> {
        self.states.values()
    }

    pub fn in_channel(&self, guild: GuildId, channel: ChannelId) -> impl Iterator<Item = &VState> {
        self.states
            .values()
            .filter(move |v| v.guild == guild && v.channel == channel)
    }

    pub fn of_user(&self, user: UserId) -> impl Iterator<Item = &VState> {
        self.states.values().filter(move |v| v.user == user)
    }
}

/// After a fresh login the bot gets one snapshot per community; the world is trustworthy once every community has
/// answered, or after `cap_s` seconds.
#[derive(Debug, Clone, Default)]
pub struct Burst {
    pending: BTreeSet<GuildId>,
    started: Option<f64>,
}

impl Burst {
    pub fn start(&mut self, guilds: impl IntoIterator<Item = GuildId>, now: f64) {
        self.pending = guilds.into_iter().collect();
        self.started = Some(now);
    }

    pub fn resolve(&mut self, guild: GuildId) {
        self.pending.remove(&guild);
    }

    pub fn done(&self, now: f64, cap_s: f64) -> bool {
        match self.started {
            None => true,
            Some(t) => self.pending.is_empty() || now - t >= cap_s,
        }
    }

    pub fn pending(&self) -> &BTreeSet<GuildId> {
        &self.pending
    }
}
