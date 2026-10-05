/// A text-to-speech voice.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceInfo {
    /// Stable id within its model, e.g. `de_DE-thorsten-high`; `<model>:<id>` names it among all models.
    pub id: String,
    /// The speech model it belongs to (`piper`, …).
    pub model: String,
    /// Language tag of the voice, e.g. `de-DE`, `en-US` (the main one when it speaks several).
    pub language: String,
    /// Every language it speaks (one for most voices; many for a multilingual model and its cloned voices).
    pub languages: Vec<String>,
    /// Speaker names when the voice has several (index = speaker id).
    pub speakers: Vec<String>,
    pub sample_rate: u32,
    /// Quality label of the voice (x_low, low, medium, high).
    pub quality: String,
}

impl VoiceInfo {
    /// `<model>:<id>`: its name among every model's voices.
    pub fn full_id(&self) -> String {
        format!("{}:{}", self.model, self.id)
    }

    /// Whether `voice` (its id, or `<model>:<id>`) names it.
    pub fn named(&self, voice: &str) -> bool {
        self.id == voice
            || voice
                .split_once(':')
                .is_some_and(|(model, id)| model == self.model && id == self.id)
    }
}

/// How to speak.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeakOpts {
    /// Speaking speed: 1.0 is the voice's own pace, 2.0 twice as fast.
    pub rate: f32,
    /// Speaker of a multi-speaker voice (`None` = the voice's default).
    pub speaker: Option<usize>,
    /// Silence between sentences, in milliseconds.
    pub sentence_gap_ms: u32,
}

impl Default for SpeakOpts {
    fn default() -> Self {
        SpeakOpts {
            rate: 1.0,
            speaker: None,
            sentence_gap_ms: 150,
        }
    }
}

/// Synthesised speech: mono samples in [-1, 1].
#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// Phonemes the voice could not pronounce (skipped), for diagnostics.
    pub unknown_phonemes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TtsError {
    #[error("no voice {0:?}")]
    NoVoice(String),
    #[error("the voice could not be loaded: {0}")]
    Load(String),
    #[error("the text could not be turned into phonemes: {0}")]
    Phonemes(String),
    #[error("the voice failed: {0}")]
    Failed(String),
    #[error("nothing to say")]
    Empty,
    #[error("this speech model cannot make voices from samples")]
    NoCloning,
    #[error("this speech model needs to know what is said in the sample")]
    NeedsTranscript,
}

/// A speech model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TtsInfo {
    /// Its name in voice ids (`piper`, …).
    pub model: String,
    /// Whether it makes voices from samples.
    pub cloning: bool,
}

/// A voice made from a sample: the model's own data (codes, embeddings, what the sample says, …), kept by the bot and
/// given back to the model at every start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClonedVoice {
    pub data: Vec<u8>,
}

/// Text-to-speech engine. Owned by one thread.
pub trait TtsEngine: Send + 'static {
    fn info(&self) -> TtsInfo;
    /// Voices currently available.
    fn voices(&self) -> Vec<VoiceInfo>;
    /// Speaks `text` with `voice`.
    fn synthesize(&mut self, voice: &str, text: &str, opts: &SpeakOpts) -> Result<Speech, TtsError>;
    /// Makes a voice from `sample` (mono, in [-1, 1], at `sample_rate`) and what is said in it, when known.
    fn clone_voice(
        &mut self,
        sample: &[f32],
        sample_rate: u32,
        transcript: Option<&str>,
    ) -> Result<ClonedVoice, TtsError> {
        let _ = (sample, sample_rate, transcript);
        Err(TtsError::NoCloning)
    }
    /// Adds a voice made by [`TtsEngine::clone_voice`] (now or at an earlier start) as `id`.
    fn add_voice(&mut self, id: &str, voice: &ClonedVoice) -> Result<VoiceInfo, TtsError> {
        let _ = (id, voice);
        Err(TtsError::NoCloning)
    }
    /// Removes a voice added with [`TtsEngine::add_voice`].
    fn remove_voice(&mut self, id: &str) -> Result<(), TtsError> {
        let _ = id;
        Err(TtsError::NoCloning)
    }
}
