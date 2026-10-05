//! Version 1 of the domain types. Never changed incompatibly; a v2 would live beside it.

mod audio;
mod ids;
mod labels;
mod misc;
mod voice;

pub use audio::{FRAME, FRAME_MS, LISTEN_RATE, PLAY_RATE};
pub use ids::{ChannelId, ConnectionId, GuildId, IdError, MessageId, RoleId, UserId, first_name, unmention};
pub use labels::{ClfLang, Label, UnknownLabel};
pub use misc::{
    ActionKind, ActionOutcome, Audience, BlobHash, HashError, Lang, LangError, PlayPurpose, Scope, ScopeKind,
    SentenceId, sha256_from_hex,
};
pub use voice::VoiceState;
