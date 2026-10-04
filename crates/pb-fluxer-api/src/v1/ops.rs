//! What the bot asks of Fluxer.

use bytes::Bytes;
use jiff::Timestamp;
use pb_domain::{ChannelId, ConnectionId, GuildId, MessageId, UserId};

/// A voice-state update (gateway op 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceStateOp {
    /// Open a new connection in a channel.
    Join { guild: GuildId, channel: ChannelId },
    /// Confirm or move an existing connection.
    Update {
        guild: GuildId,
        channel: ChannelId,
        connection: ConnectionId,
    },
    /// Leave with a connection.
    Leave { guild: GuildId, connection: ConnectionId },
}

/// Where a message goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    Channel(ChannelId),
    /// A direct message.
    User(UserId),
}

/// A file attached to a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub filename: String,
    pub media_type: String,
    pub bytes: Bytes,
}

/// A message to send. Only the listed users are pinged.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutgoingMessage {
    pub content: String,
    /// Answer this message (falls back to a plain message when it is gone or cannot be referenced).
    pub reply_to: Option<MessageId>,
    pub ping: Vec<UserId>,
    pub files: Vec<Attachment>,
}

/// A message to react to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageRef {
    pub channel: ChannelId,
    pub message: MessageId,
}

/// A change to a member (moderation).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MemberPatch {
    /// Server-mute (needs Mute Members).
    pub mute: Option<bool>,
    /// Disconnect from voice (needs Move Members).
    pub disconnect: bool,
    /// Time out until / lift the time-out (`Some(None)`; needs Moderate Members; at most 365.25 days).
    pub timeout_until: Option<Option<Timestamp>>,
    /// What Fluxer's audit log shows as the reason (a time-out keeps it in full; other changes keep its ASCII part).
    pub reason: Option<String>,
}
