//! Messages on the socket.

use pb_domain::{GuildId, UserId};
use serde::{Deserialize, Serialize};

use super::state::{TopicDelta, TopicState};

/// The protocol version in `Hello` and `Welcome`.
pub const PROTO: u32 = 1;

/// Milliseconds since the Unix epoch on the bot's clock.
pub type ServerMs = i64;

/// Something a page can watch.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Topic {
    /// Communities and their people with live dots (every page).
    Sidebar,
    /// One tile per person the bot hears, and the latest violations.
    Wall,
    /// A community: calls, tracked people, the bot's connections.
    Guild { guild: GuildId },
    /// One person in one community: presence, levels, sentences, counts.
    Person { guild: GuildId, user: UserId },
    /// The bot itself (owner only).
    System,
}

/// From the page to the bot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First message after connecting: the topics the page shows.
    Hello {
        proto: u32,
        view: u64,
        visible: bool,
        topics: Vec<Topic>,
    },
    Sub {
        view: u64,
        topic: Topic,
    },
    Unsub {
        topic: Topic,
    },
    /// The tab was hidden or shown. Shown bumps `view`; the bot answers with fresh snapshots.
    Visibility {
        view: u64,
        visible: bool,
    },
    /// The page lost track of a topic (a gap in `rev`): send a snapshot.
    Resync {
        view: u64,
        topic: Topic,
    },
    Pong {
        nonce: u64,
    },
}

/// Why a topic was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    /// This login may not see it (another community, or owner-only).
    NotAllowed,
    /// No such community or person.
    NotFound,
}

/// Why the page should reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReloadReason {
    /// The bot was updated; the page's code is out of date.
    NewVersion,
    /// The web UI address or allowed hosts changed.
    AddressChanged,
}

/// From the bot to the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ServerMsg {
    Welcome {
        proto: u32,
        server_ms: ServerMs,
        version: String,
    },
    /// The whole state of a topic at `rev`.
    Snapshot {
        view: u64,
        topic: Topic,
        rev: u64,
        state: Box<TopicState>,
    },
    /// One change; applies to `rev - 1`.
    Delta {
        view: u64,
        topic: Topic,
        rev: u64,
        delta: Box<TopicDelta>,
    },
    Denied {
        view: u64,
        topic: Topic,
        reason: DenyReason,
    },
    /// The login's rights changed: denied topics may be asked for again.
    AccessChanged,
    /// The login ended. The page shows "log in again" and does not reconnect by itself.
    AuthExpired,
    /// Answer with `Pong`. `rtt_ms` is the round trip of the previous ping.
    Ping {
        nonce: u64,
        server_ms: ServerMs,
        rtt_ms: Option<u32>,
    },
    /// The bot is stopping; reconnect after a moment if `restarting`.
    Shutdown {
        restarting: bool,
    },
    Reload {
        reason: ReloadReason,
    },
}

impl ServerMsg {
    /// The `view` a frame was made for (frames for an older view are dropped).
    pub fn view(&self) -> Option<u64> {
        match self {
            ServerMsg::Snapshot { view, .. } | ServerMsg::Delta { view, .. } | ServerMsg::Denied { view, .. } => {
                Some(*view)
            }
            _ => None,
        }
    }
}
