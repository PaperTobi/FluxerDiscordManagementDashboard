//! Version 1 of the model interfaces.

mod classifier;
#[cfg(feature = "contract-tests")]
pub mod contract;
mod tts;
mod vad;

pub use classifier::{Classifier, ClassifierInfo, ModelError, RawScores};
pub use tts::{SpeakOpts, Speech, TtsEngine, TtsError, VoiceInfo};
pub use vad::{FRAME, VadInfo, VadModel, VadState, energy_gate};

/// The rate of all audio the VAD and the classifier take; frames of [`FRAME`] samples.
pub use pb_domain::LISTEN_RATE;
