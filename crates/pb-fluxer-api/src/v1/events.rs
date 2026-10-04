//! What the gateway tells the engine (already parsed and typed).

use pb_domain::{ChannelId, GuildId, RoleId, UserId, VoiceState};

use super::model::{BotIdentity, Channel, Guild, IncomingMessage, Member, Role};
use super::voice::VoiceGrant;

/// Why the gateway stopped for good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fatal {
    /// Fluxer rejected the bot token (4004 on Identify, 401 from the API).
    TokenRejected,
    /// The bot is in more than 2,500 communities and must shard (4011).
    ShardingRequired,
    /// The gateway does not speak this protocol version (4012) or refused the shard (4010).
    Protocol { code: u16, reason: String },
}

/// A gateway event.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum GatewayEvent {
    /// Logged in (a new session: voice connections of an earlier one are gone). One `GuildAvailable` /
    /// `GuildUnavailable` per listed community follows.
    Ready {
        bot: BotIdentity,
        session: String,
        guilds: Vec<GuildId>,
    },
    /// The session was resumed: nothing was missed.
    Resumed,
    /// The connection dropped; the client reconnects by itself (`resuming`: the session and voice connections survive).
    Down {
        code: Option<u16>,
        resuming: bool,
    },
    /// The client stopped and will not reconnect.
    Stopped(Fatal),
    GuildAvailable(Box<Guild>),
    /// The community is in an outage (keep it, as unavailable).
    GuildUnavailable(GuildId),
    /// The bot left or was removed.
    GuildRemoved(GuildId),
    GuildUpdated {
        guild: GuildId,
        name: String,
        icon: Option<String>,
        owner: Option<UserId>,
    },
    RolesChanged {
        guild: GuildId,
        upsert: Vec<Role>,
        removed: Vec<RoleId>,
    },
    ChannelsChanged {
        guild: GuildId,
        upsert: Vec<Channel>,
        removed: Vec<ChannelId>,
    },
    MemberUpdated {
        guild: GuildId,
        member: Box<Member>,
    },
    MemberRemoved {
        guild: GuildId,
        user: UserId,
    },
    /// A voice state, with the member when Fluxer sent it (names of people joining voice).
    VoiceState {
        state: Box<VoiceState>,
        member: Option<Box<Member>>,
    },
    /// Where the bot may join (answer to its own voice-state update).
    VoiceServer(Box<VoiceGrant>),
    Message(Box<IncomingMessage>),
}
