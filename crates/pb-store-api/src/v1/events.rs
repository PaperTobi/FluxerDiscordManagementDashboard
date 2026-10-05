//! The kinds of events and their data. Each kind has a version `v`; a new shape gets a new version and the old ones
//! keep decoding forever.

use jiff::Timestamp;
use pb_domain::{
    ActionKind, ActionOutcome, Audience, BlobHash, ChannelId, ClfLang, GuildId, Label, Lang, MessageId, PlayPurpose,
    SentenceId, UserId,
};
use pb_settings::Change;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::envelope::{NewEvent, StoredEvent};

/// Who did something.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub via: Via,
}

impl Actor {
    pub fn system() -> Actor {
        Actor {
            user: None,
            name: None,
            via: Via::System,
        }
    }
}

/// Through what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Web,
    Chat,
    /// A hand edit of a settings file, picked up at start or on reload.
    File,
    Import,
    System,
}

// ------------------------------------------------------------------------------------------------ lifecycle

/// `bot.started` v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Started {
    pub version: String,
}

/// `bot.stopped` v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stopped {
    /// Every queued sentence was finished.
    pub clean: bool,
}

/// `log.repaired` v1: a torn tail was cut off at open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogRepaired {
    pub segment: String,
    pub cut_bytes: u64,
    pub last_good_seq: u64,
}

// ------------------------------------------------------------------------------------------------ sentences

/// Why the segmenter cut a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutCause {
    Pause,
    MaxLength,
    Muted,
    Left,
    StreamStalled,
    Shutdown,
    /// Imported from the old bot (the reason was not kept).
    Unknown,
}

/// What the bot decided about a scored sentence.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DecisionRecord {
    NothingFlagged,
    InvalidScore,
    NoLongerTracked,
    Strike {
        strike: u32,
        of: u32,
    },
    Warn {
        label: Label,
        score: f32,
        step: u32,
        count: u32,
    },
    Observe {
        label: Label,
        score: f32,
        step: u32,
        count: u32,
    },
    Late {
        label: Label,
        score: f32,
        step: u32,
        count: u32,
    },
    /// Imported from the old bot only: flagged, but its hourly warning cap (which no longer exists) held the warning
    /// back and the sentence was not counted.
    OldHourlyCap {
        per_hour: u32,
    },
}

impl DecisionRecord {
    pub fn is_violation(&self) -> bool {
        matches!(
            self,
            DecisionRecord::Warn { .. } | DecisionRecord::Observe { .. } | DecisionRecord::Late { .. }
        )
    }

    /// (label, score, step, count) of a violation.
    pub fn violation(&self) -> Option<(Label, f32, u32, u32)> {
        match *self {
            DecisionRecord::Warn {
                label,
                score,
                step,
                count,
            }
            | DecisionRecord::Observe {
                label,
                score,
                step,
                count,
            }
            | DecisionRecord::Late {
                label,
                score,
                step,
                count,
            } => Some((label, score, step, count)),
            _ => None,
        }
    }
}

/// Where a sentence record came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SentenceSource {
    Live,
    /// From the old bot's history (fewer details).
    Import,
}

/// `sentence` v1: every scored sentence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SentenceRecord {
    pub id: SentenceId,
    pub guild: GuildId,
    pub channel: ChannelId,
    pub user: UserId,
    /// When the speech started.
    pub started: Timestamp,
    pub dur_ms: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_db: Option<f32>,
    pub cut: CutCause,
    /// Per detection type, in model order.
    #[serde(with = "scores")]
    pub scores: [f32; 8],
    pub language: ClfLang,
    /// The bar each enabled type had to reach.
    pub thresholds: Vec<(Label, f32)>,
    pub flagged: Vec<Label>,
    pub decision: DecisionRecord,
    /// Counted in the swear jar.
    pub jar: bool,
    /// The recording, when kept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<BlobHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub infer_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_to_verdict_ms: Option<u32>,
    /// The classifier and its weights revision.
    pub model: String,
    pub source: SentenceSource,
}

// ------------------------------------------------------------------------------------------------ playback and actions

/// `played` v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayRecord {
    pub id: SentenceId,
    pub guild: GuildId,
    pub channel: ChannelId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<UserId>,
    pub purpose: PlayPurpose,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentence: Option<SentenceId>,
    /// The voice line key (`warning.profanity.2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<BlobHash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<Lang>,
    pub audience: Audience,
    pub started: Timestamp,
    pub dur_ms: u32,
    pub outcome: PlayOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Actor>,
}

impl PlayRecord {
    /// It reached the people it was for (in the call, or written in the chat instead).
    pub fn ok(&self) -> bool {
        matches!(self.outcome, PlayOutcome::Played | PlayOutcome::Written)
    }
}

/// How a playback went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PlayOutcome {
    Played,
    /// The bot may not speak there: written in the channel's chat instead.
    Written,
    /// It waited longer than the latest-warning setting allows.
    TooLate,
    /// No voice line, voice or clip for the person's languages.
    NothingToSay,
    /// The bot may not speak there and only records it (the setting says so, or there is no text to write).
    NotSpoken,
    /// Rendering or playing failed (the cause, for the log).
    Failed {
        error: String,
    },
}

/// `action` v1: a moderation action from an escalation step (or its timed undo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRecord {
    pub id: SentenceId,
    pub guild: GuildId,
    pub user: UserId,
    pub kind: ActionKind,
    /// How long it lasts, in seconds (a mute or a time-out).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentence: Option<SentenceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    pub outcome: ActionOutcome,
    /// When the action must be undone (a timed mute).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undo_at: Option<Timestamp>,
    /// For an undo: the action it undoes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undoes: Option<SentenceId>,
    /// For a failed undo: when it is tried again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<Timestamp>,
}

/// `jar.reset` v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JarReset {
    pub guild: GuildId,
    pub user: UserId,
    pub by: Actor,
}

/// `jar.baseline` v1: a count carried over from the old bot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JarBaseline {
    pub guild: GuildId,
    pub user: UserId,
    pub count: u64,
}

// ------------------------------------------------------------------------------------------------ settings and content

/// `settings.changed` v1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SettingsChanged {
    pub change: Change,
    pub by: Actor,
}

/// What a blob is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlobRole {
    /// An uploaded or recorded file as it arrived.
    Original,
    /// A prepared 48 kHz render of a voice clip.
    Render,
    /// A recorded sentence (16 kHz WAV).
    Recording,
    /// A speech model's data for a voice made from a sample.
    Voice,
}

/// `blob.added` v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobAdded {
    pub hash: BlobHash,
    pub size: u64,
    pub media_type: String,
    pub role: BlobRole,
}

/// `blob.deleted` v1: content removed on request (the record that it existed stays).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobDeleted {
    pub hash: BlobHash,
    pub by: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// A voice clip in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipRecord {
    /// The prepared render (what is played and referenced by voice lines).
    pub render: BlobHash,
    pub original: BlobHash,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<Lang>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    pub dur_ms: u32,
    /// The classifier's scores for the clip itself (a clip that sounds like a violation is marked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "opt_scores")]
    pub self_check: Option<[f32; 8]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heard_language: Option<ClfLang>,
    /// Who added the clip: they and the owner may change or remove it (the library is shared by every community).
    pub added_by: Actor,
    /// Who saved this version.
    pub by: Actor,
}

impl ClipRecord {
    /// Whether `user` may rename or remove this clip.
    pub fn editable_by(&self, user: UserId, owner: bool) -> bool {
        owner || self.added_by.user == Some(user)
    }
}

/// `clip.saved` v1: a clip added or its name, language or transcript changed.
pub type ClipSaved = ClipRecord;

/// `clip.removed` v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipRemoved {
    pub render: BlobHash,
    pub by: Actor,
}

/// A voice in the voice library: made by a speech model from a sample, usable wherever voices are chosen (as
/// `<model>:<id>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceRecord {
    pub model: String,
    /// Made from the name when the voice was added; it never changes.
    pub id: String,
    pub name: String,
    /// The sample as it arrived.
    pub sample: BlobHash,
    /// The model's data for the voice.
    pub data: BlobHash,
    /// What is said in the sample.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    /// Who added the voice: they and the owner may rename or remove it.
    pub added_by: Actor,
    /// Who saved this version.
    pub by: Actor,
}

impl VoiceRecord {
    /// `<model>:<id>`.
    pub fn voice_id(&self) -> String {
        format!("{}:{}", self.model, self.id)
    }

    /// Whether `user` may rename or remove this voice.
    pub fn editable_by(&self, user: UserId, owner: bool) -> bool {
        owner || self.added_by.user == Some(user)
    }
}

/// `voice.saved` v1: a voice added to the library or renamed.
pub type VoiceSaved = VoiceRecord;

/// `voice.removed` v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoiceRemoved {
    pub model: String,
    pub id: String,
    pub by: Actor,
}

/// `chat.flagged` v1: a chat message with a word or phrase of the word list, and what was decided (as for a sentence:
/// strikes, warnings and escalation count both together). A word-list match counts as profanity at score 1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRecord {
    /// The record's own id (actions and reports refer to it as to a sentence).
    pub id: SentenceId,
    pub guild: GuildId,
    pub channel: ChannelId,
    pub message: MessageId,
    pub user: UserId,
    pub at: Timestamp,
    pub text: String,
    /// The entries of the word list that were found.
    pub matches: Vec<String>,
    pub decision: DecisionRecord,
    /// It went into the swear jar.
    pub jar: bool,
}

/// `chat.deleted` v1: the bot removed (or failed to remove) a flagged message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatDeleted {
    pub id: SentenceId,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `person.seen` v1: a person's names and avatar as Fluxer reported them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonSeen {
    pub user: UserId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild: Option<GuildId>,
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nick: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

/// `community.seen` v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunitySeen {
    pub guild: GuildId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// Why a message was sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessagePurpose {
    Modlog {
        sentence: SentenceId,
    },
    OwnerDm {
        sentence: SentenceId,
    },
    Digest {
        from: Timestamp,
        until: Timestamp,
    },
    /// The warning, as a reply to a flagged chat message.
    ChatReply {
        record: SentenceId,
    },
}

/// `message.sent` v1: a mod-log post, a direct message or a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSent {
    pub purpose: MessagePurpose,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild: Option<GuildId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelId>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub with_audio: bool,
}

/// `login` v1: someone logged in to the web UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login {
    pub user: UserId,
    pub name: String,
    pub owner: bool,
}

/// `audit.imported` v1: an audit entry of the old bot, as it was recorded there (its keys and values are the old
/// bot's; nothing acts on it).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedAudit {
    pub at: Timestamp,
    pub actor: Actor,
    /// `ui`, `chat`, `system` …
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild: Option<GuildId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<UserId>,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<serde_json::Value>,
}

/// `import.done` v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportDone {
    pub from: String,
    pub sentences: u64,
    pub violations: u64,
    pub recordings: u64,
    pub clips: u64,
    pub settings: u64,
    pub audit: u64,
    /// What could not be carried over (removed caps, unreadable rows).
    pub notes: Vec<String>,
}

// ------------------------------------------------------------------------------------------------ the enum

/// Every kind of event.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Started(Started),
    Stopped(Stopped),
    LogRepaired(LogRepaired),
    Sentence(Box<SentenceRecord>),
    Played(Box<PlayRecord>),
    Action(Box<ActionRecord>),
    JarReset(JarReset),
    JarBaseline(JarBaseline),
    SettingsChanged(Box<SettingsChanged>),
    BlobAdded(BlobAdded),
    BlobDeleted(BlobDeleted),
    ClipSaved(Box<ClipSaved>),
    ClipRemoved(ClipRemoved),
    VoiceSaved(Box<VoiceSaved>),
    VoiceRemoved(VoiceRemoved),
    ChatFlagged(Box<ChatRecord>),
    ChatDeleted(ChatDeleted),
    PersonSeen(PersonSeen),
    CommunitySeen(CommunitySeen),
    MessageSent(MessageSent),
    Login(Login),
    ImportDone(ImportDone),
    ImportedAudit(Box<ImportedAudit>),
    /// A kind or version this build does not know (written by a newer bot).
    Unknown {
        kind: String,
        v: u32,
    },
}

/// The data of an event that does not match its kind.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("event {seq} ({kind} v{v}): {message}")]
pub struct DecodeError {
    pub seq: u64,
    pub kind: String,
    pub v: u32,
    pub message: String,
}

macro_rules! kinds {
    ($($variant:ident($ty:ty) = $kind:literal v $v:literal $(boxed $boxed:tt)?;)*) => {
        impl Event {
            /// The kind name in the log.
            pub fn kind(&self) -> &str {
                match self {
                    $(Event::$variant(_) => $kind,)*
                    Event::Unknown { kind, .. } => kind,
                }
            }

            /// The event to append (`None` for [`Event::Unknown`], which is never written).
            pub fn to_new(&self, ts: Option<Timestamp>) -> Option<NewEvent> {
                let (kind, v, data) = match self {
                    $(Event::$variant(x) => ($kind, $v, serde_json::to_value(x)),)*
                    Event::Unknown { .. } => return None,
                };
                match data {
                    Ok(data) => Some(NewEvent { kind: kind.to_owned(), v, ts, data }),
                    Err(e) => {
                        tracing::error!(kind, error = %e, "an event could not be written as JSON");
                        None
                    }
                }
            }

            /// Decodes a stored event.
            pub fn from_stored(e: &StoredEvent) -> Result<Event, DecodeError> {
                Event::decode(&e.kind, e.v, &e.data).map_err(|message| DecodeError {
                    seq: e.seq,
                    kind: e.kind.clone(),
                    v: e.v,
                    message,
                })
            }

            /// Decodes an event's data by its kind and version.
            pub fn decode(kind: &str, v: u32, data: &serde_json::Value) -> Result<Event, String> {
                let err = |m: serde_json::Error| m.to_string();
                match (kind, v) {
                    $(($kind, $v) => Ok(Event::$variant(kinds!(@wrap decode::<$ty>(data).map_err(err)? $(, $boxed)?))),)*
                    _ => Ok(Event::Unknown { kind: kind.to_owned(), v }),
                }
            }
        }
    };
    (@wrap $e:expr) => { $e };
    (@wrap $e:expr, $b:tt) => { Box::new($e) };
}

/// Scores as JSON numbers; one that is not a number (the model's fault) is written as `null` and read back as NaN, so
/// the event stays readable.
mod scores {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(v: &[f32; 8], s: S) -> Result<S::Ok, S::Error> {
        v.map(|x| x.is_finite().then_some(x)).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[f32; 8], D::Error> {
        Ok(<[Option<f32>; 8]>::deserialize(d)?.map(|x| x.unwrap_or(f32::NAN)))
    }
}

mod opt_scores {
    use serde::{Deserialize, Deserializer, Serializer};

    #[allow(clippy::ref_option)] // serde's `with` passes a reference to the field
    pub fn serialize<S: Serializer>(v: &Option<[f32; 8]>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(x) => super::scores::serialize(x, s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<[f32; 8]>, D::Error> {
        Ok(Option::<[Option<f32>; 8]>::deserialize(d)?.map(|v| v.map(|x| x.unwrap_or(f32::NAN))))
    }
}

fn decode<T: DeserializeOwned>(v: &serde_json::Value) -> Result<T, serde_json::Error> {
    T::deserialize(v)
}

kinds! {
    Started(Started) = "bot.started" v 1;
    Stopped(Stopped) = "bot.stopped" v 1;
    LogRepaired(LogRepaired) = "log.repaired" v 1;
    Sentence(SentenceRecord) = "sentence" v 1 boxed x;
    Played(PlayRecord) = "played" v 1 boxed x;
    Action(ActionRecord) = "action" v 1 boxed x;
    JarReset(JarReset) = "jar.reset" v 1;
    JarBaseline(JarBaseline) = "jar.baseline" v 1;
    SettingsChanged(SettingsChanged) = "settings.changed" v 1 boxed x;
    BlobAdded(BlobAdded) = "blob.added" v 1;
    BlobDeleted(BlobDeleted) = "blob.deleted" v 1;
    ClipSaved(ClipSaved) = "clip.saved" v 1 boxed x;
    ClipRemoved(ClipRemoved) = "clip.removed" v 1;
    VoiceSaved(VoiceSaved) = "voice.saved" v 1 boxed x;
    VoiceRemoved(VoiceRemoved) = "voice.removed" v 1;
    ChatFlagged(ChatRecord) = "chat.flagged" v 1 boxed x;
    ChatDeleted(ChatDeleted) = "chat.deleted" v 1;
    PersonSeen(PersonSeen) = "person.seen" v 1;
    CommunitySeen(CommunitySeen) = "community.seen" v 1;
    MessageSent(MessageSent) = "message.sent" v 1;
    Login(Login) = "login" v 1;
    ImportDone(ImportDone) = "import.done" v 1;
    ImportedAudit(ImportedAudit) = "audit.imported" v 1 boxed x;
}

#[cfg(test)]
mod score_tests {
    #[derive(serde::Serialize, serde::Deserialize)]
    struct S {
        #[serde(with = "super::scores")]
        s: [f32; 8],
    }

    #[test]
    fn a_score_that_is_not_a_number_stays_readable() {
        let v = serde_json::to_value(S {
            s: [0.5, f32::NAN, 0.0, 0.0, 0.0, 0.0, 0.0, f32::INFINITY],
        })
        .unwrap_or_default();
        assert_eq!(v["s"][1], serde_json::Value::Null);
        let back: S = serde_json::from_value(v).unwrap_or(S { s: [0.0; 8] });
        assert!((back.s[0] - 0.5).abs() < f32::EPSILON);
        assert!(back.s[1].is_nan() && back.s[7].is_nan());
    }
}
