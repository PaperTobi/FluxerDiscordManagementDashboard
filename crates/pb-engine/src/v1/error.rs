//! Why something asked of the engine (from the web UI) did not happen. The web UI words each case in the page's
//! language; the texts here are for logs.

use pb_audio::AudioError;
use pb_domain::{BlobHash, Lang};
use pb_fluxer_api::{FluxerError, LoginError};
use pb_infer::InferError;
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
    #[error("no such voice")]
    NoSuchVoice,
    /// A voice could not be made from the sample or given to its model.
    #[error("the voice could not be made: {0}")]
    Voice(#[from] VoiceError),
    /// The file is not audio the bot can read.
    #[error("not audio the bot can read: {0}")]
    Unreadable(AudioError),
    #[error("no such sentence")]
    NoSuchSentence,
    #[error("this sentence has no recording")]
    NoRecording,
    /// The event log stopped writing (see the System page).
    #[error("the event log is not writing")]
    LogHalted,
    /// Rendering speech or a clip failed.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// Fluxer refused or could not be reached.
    #[error("Fluxer: {0}")]
    Fluxer(#[from] FluxerError),
    /// A Fluxer instance could not be used for logging in (the address, or Fluxer cannot be reached).
    #[error(transparent)]
    Login(#[from] LoginError),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Why a voice could not be made from a sample.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum VoiceError {
    #[error("this speech model cannot make voices from samples")]
    NoCloning,
    #[error("this speech model needs to know what is said in the sample")]
    NeedsTranscript,
    #[error("the speech model {0:?} does not run")]
    NoModel(String),
    #[error("{0}")]
    Failed(String),
}

impl From<InferError> for VoiceError {
    fn from(e: InferError) -> Self {
        match e {
            InferError::Tts(pb_models_api::TtsError::NoCloning) => VoiceError::NoCloning,
            InferError::Tts(pb_models_api::TtsError::NeedsTranscript) => VoiceError::NeedsTranscript,
            InferError::NoModel(m) => VoiceError::NoModel(m),
            other => VoiceError::Failed(other.to_string()),
        }
    }
}

/// Why speech or a clip could not be made ready to play.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RenderError {
    #[error("no voice speaks {0}")]
    NoVoice(Lang),
    #[error("text-to-speech failed: {0}")]
    Tts(#[from] InferError),
    #[error("clip {0} is missing")]
    ClipMissing(BlobHash),
    #[error("clip {clip} cannot be played: {error}")]
    ClipUnreadable { clip: BlobHash, error: AudioError },
    #[error(transparent)]
    Store(#[from] StoreError),
}
