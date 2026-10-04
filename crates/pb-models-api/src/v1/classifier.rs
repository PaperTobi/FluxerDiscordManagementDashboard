use std::num::NonZeroUsize;

use pb_domain::{ClfLang, Label};

/// What a classifier is and what it accepts.
#[derive(Debug, Clone, PartialEq)]
pub struct ClassifierInfo {
    /// Model name and revision, e.g. `Roblox/voice-safety-classifier-v3@ddb1ffb0…`.
    pub model: String,
    /// Shortest input in samples the model can score.
    pub min_samples: usize,
    /// Longest input in samples the model reads in one pass; longer audio has to be split by the caller.
    pub max_samples: usize,
    /// Where the model runs, for the System page (e.g. "CPU, 4 threads" or "Vulkan: AMD Radeon RX 9070 XT").
    pub device: String,
}

/// One pass of the classifier over one clip.
#[derive(Debug, Clone, PartialEq)]
pub struct RawScores {
    /// Independent probabilities (sigmoid) per label, indexed by [`Label::index`].
    pub labels: [f32; 8],
    /// Language probabilities (softmax), indexed by [`ClfLang::index`].
    pub languages: [f32; 30],
}

impl RawScores {
    pub fn label(&self, label: Label) -> f32 {
        self.labels[label.index()]
    }

    /// The most likely spoken language.
    pub fn top_language(&self) -> ClfLang {
        let mut best = 0;
        for (i, p) in self.languages.iter().enumerate() {
            if *p > self.languages[best] {
                best = i;
            }
        }
        ClfLang::ALL[best]
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ModelError {
    #[error("the clip is too short: {samples} samples, at least {min} are needed")]
    TooShort { samples: usize, min: usize },
    #[error("the clip is too long for one pass: {samples} samples, at most {max}")]
    TooLong { samples: usize, max: usize },
    #[error("could not load the model: {0}")]
    Load(String),
    #[error("the model failed: {0}")]
    Failed(String),
}

/// The voice-safety classifier: one clip in, label and language probabilities out.
pub trait Classifier: Send + 'static {
    fn info(&self) -> &ClassifierInfo;
    /// Scores one clip of mono 16 kHz audio in [-1, 1] (int16 / 32768). Input outside
    /// `min_samples..=max_samples` is an error, never silently cut.
    fn classify(&mut self, pcm16k: &[f32]) -> Result<RawScores, ModelError>;
    /// How many CPU threads the model may use from the next clip on.
    fn set_threads(&mut self, threads: NonZeroUsize) -> Result<(), ModelError>;
}
