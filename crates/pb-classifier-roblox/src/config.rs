//! The model's `config.json`, checked against the architecture this crate implements.

use std::path::Path;

use pb_domain::{ClfLang, Label};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ModelConfig {
    pub model_type: String,
    pub hidden_size: usize,
    pub num_attention_heads: usize,
    pub num_hidden_layers: usize,
    pub classifier_proj_size: usize,
    /// `[layer index, pooling ratio]` pairs: average-pool the sequence after that layer.
    pub time_reduction: Vec<[usize; 2]>,
    pub n_mels: usize,
    pub n_fft: usize,
    pub hop_length: usize,
    pub sample_rate: u32,
    pub max_positions: usize,
    pub num_labels: usize,
    pub num_language_heads: usize,
    pub labels: Vec<String>,
    pub languages: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("reading {path}: {source}")]
    Read { path: String, source: std::io::Error },
    #[error("parsing {path}: {source}")]
    Parse { path: String, source: serde_json::Error },
    #[error("unsupported model: {0}")]
    Unsupported(String),
}

impl ModelConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        let cfg: ModelConfig = serde_json::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        cfg.check()?;
        Ok(cfg)
    }

    /// The label and language order must be the one the domain types use, and the shapes must fit this code.
    fn check(&self) -> Result<(), ConfigError> {
        let fail = |what: String| Err(ConfigError::Unsupported(what));
        if self.model_type != "VoiceToxicityClassifier" {
            return fail(format!("model_type {:?}", self.model_type));
        }
        let labels: Vec<&str> = Label::ALL.iter().map(|l| l.model_name()).collect();
        if self.labels != labels || self.num_labels != labels.len() {
            return fail(format!("labels {:?}", self.labels));
        }
        let langs: Vec<&str> = ClfLang::ALL.iter().map(|l| l.code()).collect();
        if self.languages != langs || self.num_language_heads != langs.len() {
            return fail(format!("languages {:?}", self.languages));
        }
        if !self.hidden_size.is_multiple_of(self.num_attention_heads) || !self.classifier_proj_size.is_multiple_of(16) {
            return fail("hidden sizes not divisible by the head counts".into());
        }
        if self.sample_rate != pb_models_api::LISTEN_RATE {
            return fail(format!("sample rate {}", self.sample_rate));
        }
        if self.n_fft < 2 || self.hop_length == 0 || self.hop_length > self.n_fft {
            return fail(format!("frames of {} samples every {}", self.n_fft, self.hop_length));
        }
        if self
            .time_reduction
            .iter()
            .any(|[layer, ratio]| *layer >= self.num_hidden_layers || *ratio == 0)
        {
            return fail(format!("time_reduction {:?}", self.time_reduction));
        }
        Ok(())
    }

    /// Longest input the model reads (`max_positions` tokens after the stride-2 convolution).
    pub fn max_samples(&self) -> usize {
        self.max_positions * self.hop_length * 2
    }
}
