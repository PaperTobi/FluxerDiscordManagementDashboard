//! The query index: derived from the log, rebuilt from it at will, never the source of truth.

use async_trait::async_trait;
use jiff::Timestamp;
use jiff::civil::Date;
use pb_domain::{GuildId, Label, SentenceId, UserId};
use serde::{Deserialize, Serialize};

use super::events::{ActionRecord, ClipRecord, CommunitySeen, Event, SentenceRecord, VoiceRecord};
use super::log::StoreError;

/// Where the next page starts (pages go back in time).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Cursor(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<Cursor>,
}

impl<T> Default for Page<T> {
    fn default() -> Self {
        Page {
            items: Vec::new(),
            next: None,
        }
    }
}

/// Which sentences.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SentenceKind {
    #[default]
    All,
    Flagged,
    Violations,
    /// With a recording that is still there.
    WithAudio,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SentenceFilter {
    /// `None`: every community; otherwise only these.
    pub guilds: Option<Vec<GuildId>>,
    pub user: Option<UserId>,
    pub since: Option<Timestamp>,
    pub until: Option<Timestamp>,
    pub kind: SentenceKind,
    /// Violations of this type only.
    pub label: Option<Label>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SentenceRow {
    pub seq: u64,
    pub record: SentenceRecord,
    /// The recording was deleted on request.
    pub audio_deleted: bool,
}

/// One day of one person (in the reporting time zone).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayRow {
    pub day: Date,
    pub sentences: u32,
    pub flagged: u32,
    pub violations: u32,
    pub speech_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JarRow {
    pub guild: GuildId,
    pub user: UserId,
    pub count: u64,
}

/// What the audit page lists.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AuditFilter {
    /// `None`: every community (and what belongs to none); otherwise only these.
    pub guilds: Option<Vec<GuildId>>,
    pub user: Option<UserId>,
    /// Event kinds; empty = every audited kind.
    pub kinds: Vec<String>,
}

/// The event kinds the audit page shows.
pub const AUDIT_KINDS: &[&str] = &[
    "settings.changed",
    "action",
    "jar.reset",
    "jar.baseline",
    "blob.deleted",
    "clip.saved",
    "clip.removed",
    "voice.saved",
    "voice.removed",
    "login",
    "import.done",
    "audit.imported",
    "log.repaired",
    "bot.started",
    "bot.stopped",
    "message.sent",
];

#[derive(Debug, Clone, PartialEq)]
pub struct AuditRow {
    pub seq: u64,
    pub ts: Timestamp,
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipRow {
    pub seq: u64,
    pub added: Timestamp,
    pub record: ClipRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceRow {
    pub seq: u64,
    pub added: Timestamp,
    pub record: VoiceRecord,
}

/// The names Fluxer last reported for a person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonName {
    pub user: UserId,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The nickname in the community asked about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nick: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

impl PersonName {
    /// Nickname, else display name, else user name (empty names count as none).
    pub fn shown(&self) -> &str {
        pb_domain::first_name([self.nick.as_deref(), self.display_name.as_deref()]).unwrap_or(&self.username)
    }
}

/// The summary reports so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LastDigest {
    /// The end of the newest period a report was tried for (delivered or not).
    pub tried: Option<Timestamp>,
    /// The end of the newest period a report was delivered for.
    pub sent: Option<Timestamp>,
    /// Why the newest report could not be delivered (`None` when it was).
    pub error: Option<String>,
}

/// One person in a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestRow {
    pub guild: GuildId,
    pub user: UserId,
    pub violations: u32,
    /// The type most of the violations were.
    pub top_label: Label,
    pub max_step: u32,
    pub actions: u32,
}

#[async_trait]
pub trait Index: Send + Sync + 'static {
    /// The last event applied (0 = none).
    fn applied(&self) -> u64;

    /// Waits until every event up to `seq` is in the index.
    async fn caught_up(&self, seq: u64);

    /// What keeps the index from applying events right now (it keeps trying), if anything.
    fn problem(&self) -> Option<String>;

    /// Events of the log the index could not read (written by a newer version of the bot) and left out.
    async fn skipped(&self) -> Result<u64, StoreError>;

    /// Sentences, newest first. `page` is the page size the caller wants.
    async fn sentences(
        &self,
        f: &SentenceFilter,
        cursor: Option<Cursor>,
        page: u32,
    ) -> Result<Page<SentenceRow>, StoreError>;

    async fn sentence(&self, id: SentenceId) -> Result<Option<SentenceRow>, StoreError>;

    /// Per-day totals of one person from `from` to `until` (inclusive) in the time zone `tz` (an IANA name).
    async fn days(
        &self,
        guild: GuildId,
        user: UserId,
        from: Date,
        until: Date,
        tz: &str,
    ) -> Result<Vec<DayRow>, StoreError>;

    /// Swear jar counts (of a community, or all), highest first.
    async fn jar(&self, guild: Option<GuildId>) -> Result<Vec<JarRow>, StoreError>;

    /// When each violation happened, oldest first (seeds the escalation counts at start).
    async fn violation_times(&self) -> Result<Vec<(GuildId, UserId, Timestamp)>, StoreError>;

    /// Timed actions not undone yet (a mute to lift), by due time.
    async fn pending_undos(&self) -> Result<Vec<ActionRecord>, StoreError>;

    /// Audited events, newest first.
    async fn audit(&self, f: &AuditFilter, cursor: Option<Cursor>, page: u32) -> Result<Page<AuditRow>, StoreError>;

    /// The clip library (removed clips left out), newest first.
    async fn clips(&self) -> Result<Vec<ClipRow>, StoreError>;

    /// The voice library (removed voices left out), oldest first.
    async fn voices(&self) -> Result<Vec<VoiceRow>, StoreError>;

    /// Names of people (nicknames from `guild` when given).
    async fn people(&self, users: &[UserId], guild: Option<GuildId>) -> Result<Vec<PersonName>, StoreError>;

    async fn communities(&self) -> Result<Vec<CommunitySeen>, StoreError>;

    /// When the last report was sent.
    async fn last_digest(&self) -> Result<LastDigest, StoreError>;

    /// Per person: violations from `from` to `until`.
    async fn digest(&self, from: Timestamp, until: Timestamp) -> Result<Vec<DigestRow>, StoreError>;
}
