//! Fluxer's objects, as far as the bot reads them.

use jiff::Timestamp;
use pb_domain::{ChannelId, GuildId, MessageId, RoleId, UserId, VoiceState};

/// An account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub username: String,
    /// The display name, when set.
    pub global_name: Option<String>,
    /// Avatar hash (see [`super::Endpoints::avatar_url`]).
    pub avatar: Option<String>,
    pub bot: bool,
}

impl User {
    /// Display name, else user name.
    pub fn shown(&self) -> &str {
        self.global_name
            .as_deref()
            .filter(|n| !n.is_empty())
            .unwrap_or(&self.username)
    }
}

/// A member of a community.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// `None` when Fluxer sent only the id (members in the start-up burst, message authors).
    pub user: Option<User>,
    pub id: UserId,
    pub nick: Option<String>,
    pub roles: Vec<RoleId>,
    pub mute: bool,
    pub deaf: bool,
    pub timed_out_until: Option<Timestamp>,
}

impl Member {
    /// Nickname, else display name, else user name, else the id.
    pub fn shown(&self) -> String {
        self.nick
            .clone()
            .filter(|n| !n.is_empty())
            .or_else(|| self.user.as_ref().map(|u| u.shown().to_owned()))
            .unwrap_or_else(|| self.id.to_string())
    }
}

/// A role and what it may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Role {
    pub id: RoleId,
    pub name: String,
    pub permissions: u64,
    pub position: i64,
}

/// Who a channel permission overwrite is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwriteKind {
    Role,
    Member,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overwrite {
    /// A role id or a user id.
    pub id: u64,
    pub kind: OverwriteKind,
    pub allow: u64,
    pub deny: u64,
}

/// What a channel in a community is (Fluxer's channel `type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    Text,
    /// A voice channel (with its own chat).
    Voice,
    Category,
    /// A link in the channel list.
    Link,
    /// A type this client does not know yet.
    Other(i64),
}

impl ChannelKind {
    /// From Fluxer's type number.
    pub fn from_code(code: i64) -> ChannelKind {
        match code {
            0 => ChannelKind::Text,
            2 => ChannelKind::Voice,
            4 => ChannelKind::Category,
            998 => ChannelKind::Link,
            other => ChannelKind::Other(other),
        }
    }

    /// Whether the channel holds messages.
    pub fn holds_messages(self) -> bool {
        matches!(self, ChannelKind::Text | ChannelKind::Voice)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: ChannelId,
    pub name: String,
    pub kind: ChannelKind,
    pub parent: Option<ChannelId>,
    pub position: i64,
    pub overwrites: Vec<Overwrite>,
}

/// A community as GUILD_CREATE delivers it (roles, channels and voice states complete; members only the bot and the
/// people in voice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guild {
    pub id: GuildId,
    pub name: String,
    pub icon: Option<String>,
    pub owner: Option<UserId>,
    pub roles: Vec<Role>,
    pub channels: Vec<Channel>,
    pub members: Vec<Member>,
    pub voice_states: Vec<VoiceState>,
}

/// A chat message the bot received.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    pub id: MessageId,
    pub channel: ChannelId,
    pub guild: Option<GuildId>,
    pub author: User,
    /// The author's roles in the community (from the message's member).
    pub author_roles: Vec<RoleId>,
    pub author_nick: Option<String>,
    pub content: String,
    /// Sent by a webhook (never a command).
    pub webhook: bool,
    pub mentions: Vec<UserId>,
}

/// The bot's application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    pub id: u64,
    pub name: String,
    pub owner: Option<User>,
    /// The OAuth2 redirect addresses registered for it (logins can only come back to one of these).
    pub redirect_uris: Vec<String>,
}

/// Who the bot is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotIdentity {
    pub user: User,
    /// The application (its id is the OAuth2 client id).
    pub application: u64,
    pub owner: Option<UserId>,
}
