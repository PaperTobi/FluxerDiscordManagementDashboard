//! Scopes, languages, who hears a warning, moderation actions, content hashes, sentence ids.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::ids::{GuildId, UserId};

/// Where a setting or voice line is set. A person is always "a person in a community".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum Scope {
    Global,
    Server { guild: GuildId },
    Person { guild: GuildId, user: UserId },
}

impl Scope {
    pub fn guild(self) -> Option<GuildId> {
        match self {
            Scope::Global => None,
            Scope::Server { guild } | Scope::Person { guild, .. } => Some(guild),
        }
    }

    pub fn kind(self) -> ScopeKind {
        match self {
            Scope::Global => ScopeKind::Global,
            Scope::Server { .. } => ScopeKind::Server,
            Scope::Person { .. } => ScopeKind::Person,
        }
    }
}

/// The kind of a [`Scope`], for "may be set at" rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Global,
    Server,
    Person,
}

/// A language tag: a lower-case language (2–3 letters) and an optional upper-case region, e.g. `de`, `en-US`,
/// `pt-BR`. Accepts `_` as separator and any case.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lang {
    language: String,
    region: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a language tag: {0:?} (use e.g. de, en-US)")]
pub struct LangError(pub String);

impl Lang {
    pub fn language(&self) -> &str {
        &self.language
    }

    /// The same language without the region (`en-US` → `en`).
    pub fn base(&self) -> Lang {
        Lang {
            language: self.language.clone(),
            region: None,
        }
    }
}

impl FromStr for Lang {
    type Err = LangError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim().replace('_', "-");
        let mut parts = t.split('-');
        let language = parts.next().unwrap_or_default().to_ascii_lowercase();
        let region = parts.next().map(str::to_ascii_uppercase);
        let ok_lang = (2..=3).contains(&language.len()) && language.bytes().all(|b| b.is_ascii_lowercase());
        let ok_region = region.as_ref().is_none_or(|r| {
            (r.len() == 2 && r.bytes().all(|b| b.is_ascii_uppercase()))
                || (r.len() == 3 && r.bytes().all(|b| b.is_ascii_digit()))
        });
        if !ok_lang || !ok_region || parts.next().is_some() {
            return Err(LangError(s.to_owned()));
        }
        Ok(Lang { language, region })
    }
}

impl fmt::Display for Lang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.region {
            Some(r) => write!(f, "{}-{r}", self.language),
            None => f.write_str(&self.language),
        }
    }
}

impl Serialize for Lang {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Lang {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// Who hears the bot's voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Audience {
    /// Only the person it is about.
    Offender,
    /// Every tracked person in the call.
    Tracked,
    /// Everyone in the call.
    Channel,
}

/// Why the bot says something in a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayPurpose {
    Warning,
    StrikeNotice,
    ActionNotice,
    Greeting,
    /// "Say now" from the web UI.
    Say,
}

/// A moderation action on a member.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Server-mute the microphone (lifted again after the step's duration).
    Mute,
    /// Lift a server mute.
    Unmute,
    /// Disconnect from voice.
    Disconnect,
    /// Time out (no chatting, speaking or joining) for the step's duration.
    Timeout,
}

/// How a moderation action went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum ActionOutcome {
    Done,
    /// Actions are switched off.
    SkippedOff,
    /// Observe-only: nothing is done.
    SkippedObserve,
    /// Someone else had muted them already: left as it is (and never lifted by the bot).
    AlreadyMuted,
    /// The bot was not connected to Fluxer.
    NotConnected,
    /// Fluxer refused: the bot lacks this permission (its stable name, as in `perms::NAMES` of the Fluxer API crate).
    NotAllowed {
        permission: String,
    },
    Failed {
        error: String,
    },
}

/// SHA-256 of stored content (clips, recordings), as lower-case hex.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobHash([u8; 32]);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a sha256 content hash: {0:?}")]
pub struct HashError(pub String);

impl BlobHash {
    pub const fn from_bytes(b: [u8; 32]) -> Self {
        BlobHash(b)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl fmt::Debug for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BlobHash({})", &self.hex()[..12])
    }
}

impl fmt::Display for BlobHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", self.hex())
    }
}

/// 32 bytes from 64 hex digits (nothing else: no sign, no space).
pub fn sha256_from_hex(hex: &str) -> Option<[u8; 32]> {
    let digit = |b: u8| char::from(b).to_digit(16);
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, pair) in hex.as_bytes().chunks(2).enumerate() {
        let byte = (digit(pair[0])? * 16 + digit(pair[1])?) as u8;
        out[i] = byte;
    }
    Some(out)
}

impl FromStr for BlobHash {
    type Err = HashError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        sha256_from_hex(s.strip_prefix("sha256:").unwrap_or(s))
            .map(BlobHash)
            .ok_or_else(|| HashError(s.to_owned()))
    }
}

impl Serialize for BlobHash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for BlobHash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// One scored sentence (time-ordered UUIDv7, unique across restarts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SentenceId(pub uuid::Uuid);

impl SentenceId {
    /// A new id from the current time and randomness.
    pub fn new() -> Self {
        SentenceId(uuid::Uuid::now_v7())
    }
}

impl Default for SentenceId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SentenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl FromStr for SentenceId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        uuid::Uuid::parse_str(s.trim()).map(SentenceId)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_hex_digits_only() {
        let h = "a".repeat(64);
        assert!(h.parse::<BlobHash>().is_ok());
        assert!(format!("sha256:{h}").parse::<BlobHash>().is_ok());
        let signed = format!("+f{}", "a".repeat(62));
        assert!(signed.parse::<BlobHash>().is_err(), "a sign is not a digit");
    }

    #[test]
    fn parses_language_tags() {
        assert_eq!("de".parse::<Lang>().expect("de").to_string(), "de");
        assert_eq!("en_us".parse::<Lang>().expect("en_us").to_string(), "en-US");
        assert_eq!("es-419".parse::<Lang>().expect("es-419").to_string(), "es-419");
        for bad in ["", "german", "d", "en-USA", "en-US-x"] {
            assert!(bad.parse::<Lang>().is_err(), "{bad}");
        }
    }

    #[test]
    fn hashes_round_trip() {
        let h: BlobHash = format!("sha256:{}", "ab".repeat(32)).parse().expect("hash");
        assert_eq!(h.to_string(), format!("sha256:{}", "ab".repeat(32)));
        assert!("sha256:12".parse::<BlobHash>().is_err());
    }

    #[test]
    fn sentence_ids_round_trip() {
        let a = SentenceId::new();
        assert_eq!(a.to_string().parse::<SentenceId>().ok(), Some(a));
        assert!("nope".parse::<SentenceId>().is_err());
    }

    #[test]
    fn sentence_ids_are_ordered() {
        let a = SentenceId::new();
        let b = SentenceId::new();
        assert!(a < b);
    }
}
