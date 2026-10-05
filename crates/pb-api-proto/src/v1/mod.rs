//! Version 1, served under `/api/v1`. Ids are strings (Fluxer's snowflakes do not fit a JavaScript number); times are
//! RFC 3339.

use hmac::{Hmac, KeyInit, Mac};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// The path every v1 route starts with.
pub const BASE: &str = "/api/v1";

/// What a token may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Communities, people's shown names and avatars, swear-jar counts.
    Leaderboard,
    /// Violations: when, who, which type, step and decision (no recordings, no text).
    Violations,
    /// Also what was written (flagged chat messages) and transcripts.
    Details,
    /// Totals per community and day.
    Stats,
}

impl Scope {
    pub const ALL: [Scope; 4] = [Scope::Leaderboard, Scope::Violations, Scope::Details, Scope::Stats];

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Leaderboard => "leaderboard",
            Scope::Violations => "violations",
            Scope::Details => "details",
            Scope::Stats => "stats",
        }
    }

    pub fn parse(s: &str) -> Option<Scope> {
        Scope::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

/// A community the bot is in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Community {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
}

/// A person, as the community shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Person {
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
}

/// One place on a leaderboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Place {
    /// 1 for the most; people with the same count share a place.
    pub rank: u32,
    pub person: Person,
    pub count: u64,
}

/// A community's swear jar, most first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Leaderboard {
    pub community: Community,
    pub places: Vec<Place>,
}

/// Where a violation happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Call,
    Chat,
}

/// A violation, in a call or in the chat.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Violation {
    pub id: String,
    pub at: Timestamp,
    pub community: String,
    pub channel: String,
    pub person: Person,
    pub source: Source,
    /// The detection type (`profanity`, `harassment`, …); a word-list match in the chat is `profanity`.
    pub label: String,
    /// The violation's number within the escalation window, and the escalation step it reached.
    pub count: u32,
    pub step: u32,
    /// `warn`, `observe` (silent) or `late` (scored too late to warn).
    pub decision: String,
    /// What was written (chat), with the `details` scope only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

/// One page of violations, newest first; ask again with `cursor=<next>` for the ones before.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Violations {
    pub items: Vec<Violation>,
    pub next: Option<String>,
}

/// Totals of one community on one day (in the reporting time zone).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DayTotal {
    pub community: String,
    /// `YYYY-MM-DD`.
    pub day: String,
    pub violations: u64,
    pub people: u64,
}

/// An error answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ApiError {
    /// `unauthorized`, `forbidden`, `not_found`, `bad_request` or `unavailable`.
    pub error: String,
    pub message: String,
}

/// What a webhook receives: one event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Delivery {
    /// The event's number in the bot's log: deliveries come in this order, and again after a failure.
    pub seq: u64,
    pub kind: EventKind,
    pub at: Timestamp,
    pub community: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub data: serde_json::Value,
}

/// The kinds of events a webhook can ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    /// A violation in a call or in the chat (as in `Violation`).
    Violation,
    /// A moderation action (mute, disconnect, time-out, and the lifting of a mute).
    Action,
    /// The swear jar of a person was emptied.
    JarReset,
}

impl EventKind {
    pub const ALL: [EventKind; 3] = [EventKind::Violation, EventKind::Action, EventKind::JarReset];

    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Violation => "violation",
            EventKind::Action => "action",
            EventKind::JarReset => "jar_reset",
        }
    }

    pub fn parse(s: &str) -> Option<EventKind> {
        EventKind::ALL.into_iter().find(|x| x.as_str() == s)
    }
}

/// The header with a delivery's signature.
pub const SIGNATURE_HEADER: &str = "X-PB-Signature";
/// The header with a delivery's event number.
pub const DELIVERY_HEADER: &str = "X-PB-Delivery";

/// The signature of a delivery's body: `sha256=` and the hex HMAC-SHA256 with the webhook's secret.
pub fn sign(secret: &[u8], body: &[u8]) -> String {
    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret) else {
        // HMAC takes keys of any length.
        return String::new();
    };
    mac.update(body);
    let hex: String = mac.finalize().into_bytes().iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256={hex}")
}

/// Whether `signature` (the header's value) belongs to `body` (compared in constant time).
pub fn verify(secret: &[u8], body: &[u8], signature: &str) -> bool {
    let want = sign(secret, body);
    want.len() == signature.len()
        && want
            .bytes()
            .zip(signature.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_check_out() {
        let sig = sign(b"secret", b"{\"seq\":1}");
        assert!(sig.starts_with("sha256=") && sig.len() == 7 + 64);
        assert!(verify(b"secret", b"{\"seq\":1}", &sig));
        assert!(!verify(b"other", b"{\"seq\":1}", &sig));
        assert!(!verify(b"secret", b"{\"seq\":2}", &sig));
    }

    #[test]
    fn names_round_trip() {
        for s in Scope::ALL {
            assert_eq!(Scope::parse(s.as_str()), Some(s));
            assert_eq!(serde_json::to_value(s).unwrap_or_default(), s.as_str());
        }
        for k in EventKind::ALL {
            assert_eq!(EventKind::parse(k.as_str()), Some(k));
        }
    }
}
