//! Voice states (who is in which voice channel), as Fluxer reports them.

use serde::{Deserialize, Serialize};

use super::ids::{ChannelId, ConnectionId, GuildId, UserId};

/// One voice connection of one account in a community voice channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceState {
    pub guild: GuildId,
    /// `None` = the connection left voice.
    pub channel: Option<ChannelId>,
    pub user: UserId,
    pub connection: ConnectionId,
    /// The gateway session that opened the connection.
    pub session: Option<String>,
    pub self_mute: bool,
    pub self_deaf: bool,
    pub mute: bool,
    pub deaf: bool,
    pub suppress: bool,
    /// The client supports end-to-end encrypted voice.
    pub e2ee_capable: bool,
    /// Fluxer's version counter for this state (0 = unknown).
    pub version: u64,
}
