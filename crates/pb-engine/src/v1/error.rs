//! Why something asked of the engine (from the web UI) did not happen. The web UI words each case in the page's
//! language; the texts here are for logs.

use pb_store_api::{PlayOutcome, StoreError};

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineError {
    #[error("the bot is not connected to Fluxer")]
    NotConnected,
    #[error("they are not in a call")]
    NotInCall,
    #[error("the bot is not in that call")]
    BotNotInCall,
    #[error("no language is set for them")]
    NoLanguage,
    /// Nothing was said (too late, nothing to say, the bot may not speak there, or it failed).
    #[error("nothing was said: {0:?}")]
    NotSaid(PlayOutcome),
    #[error("no such clip")]
    NoSuchClip,
    /// The file is not audio the bot can read (the decoder's words).
    #[error("not audio the bot can read: {0}")]
    Unreadable(String),
    #[error("no such sentence")]
    NoSuchSentence,
    #[error("this sentence has no recording")]
    NoRecording,
    /// The event log stopped writing (see the System page).
    #[error("the event log is not writing")]
    LogHalted,
    /// Rendering speech or a clip failed (the cause).
    #[error("rendering failed: {0}")]
    Render(String),
    /// Fluxer refused or could not be reached (its words).
    #[error("Fluxer: {0}")]
    Fluxer(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}
