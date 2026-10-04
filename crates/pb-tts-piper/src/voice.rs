//! One Piper voice: its `.onnx.json` config and its VITS model on rten.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;

use rten::{Model, RunOptions};
use rten_tensor::prelude::*;
use rten_tensor::{NdTensor, Tensor};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct RawConfig {
    audio: Audio,
    #[serde(default)]
    inference: Inference,
    espeak: Espeak,
    #[serde(default)]
    language: Option<Language>,
    #[serde(default = "one")]
    num_speakers: usize,
    #[serde(default)]
    speaker_id_map: HashMap<String, usize>,
    #[serde(default = "espeak_type")]
    phoneme_type: String,
    phoneme_id_map: HashMap<String, Vec<i64>>,
    #[serde(default)]
    vowel_clusters: Option<Vec<Vec<String>>>,
}

fn one() -> usize {
    1
}

fn espeak_type() -> String {
    "espeak".into()
}

#[derive(Debug, Clone, Deserialize)]
struct Audio {
    sample_rate: u32,
    #[serde(default)]
    quality: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Inference {
    noise_scale: f32,
    length_scale: f32,
    noise_w: f32,
}

impl Default for Inference {
    fn default() -> Self {
        // piper-tts' defaults
        Inference {
            noise_scale: 0.667,
            length_scale: 1.0,
            noise_w: 0.8,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct Espeak {
    voice: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Language {
    #[serde(default)]
    code: Option<String>,
}

/// The parts of a voice's config that synthesis needs.
#[derive(Debug, Clone)]
pub struct VoiceConfig {
    pub sample_rate: u32,
    pub quality: String,
    pub language: String,
    pub espeak_voice: String,
    pub noise_scale: f32,
    pub length_scale: f32,
    pub noise_w: f32,
    pub speakers: Vec<String>,
    pub phoneme_id_map: HashMap<String, Vec<i64>>,
    pub vowel_clusters: Option<BTreeSet<Vec<String>>>,
}

impl VoiceConfig {
    pub fn parse(json: &str) -> Result<Self, String> {
        let raw: RawConfig = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if raw.phoneme_type != "espeak" {
            return Err(format!(
                "phoneme type {:?} is not supported (only espeak voices)",
                raw.phoneme_type
            ));
        }
        let mut speakers = vec![String::new(); raw.num_speakers.max(1)];
        for (name, id) in raw.speaker_id_map {
            if let Some(slot) = speakers.get_mut(id) {
                *slot = name;
            }
        }
        let language = raw
            .language
            .and_then(|l| l.code)
            .map(|c| c.replace('_', "-"))
            .unwrap_or_else(|| raw.espeak.voice.clone());
        Ok(VoiceConfig {
            sample_rate: raw.audio.sample_rate,
            quality: raw.audio.quality.unwrap_or_default(),
            language,
            espeak_voice: raw.espeak.voice,
            noise_scale: raw.inference.noise_scale,
            length_scale: raw.inference.length_scale,
            noise_w: raw.inference.noise_w,
            speakers,
            phoneme_id_map: raw.phoneme_id_map,
            vowel_clusters: raw.vowel_clusters.map(|v| v.into_iter().collect()),
        })
    }
}

/// Noise and length scales for one synthesis (Piper's `scales` input).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scales {
    pub noise_scale: f32,
    pub length_scale: f32,
    pub noise_w: f32,
}

/// A loaded voice.
pub struct Voice {
    pub id: String,
    pub config: VoiceConfig,
    model: Model,
}

impl std::fmt::Debug for Voice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Voice").field("id", &self.id).finish_non_exhaustive()
    }
}

impl Voice {
    /// Loads `<dir>/<id>.onnx` and `<dir>/<id>.onnx.json` (Piper's file layout).
    pub fn load(dir: &Path, id: &str) -> Result<Self, String> {
        let onnx = dir.join(format!("{id}.onnx"));
        let json = std::fs::read_to_string(dir.join(format!("{id}.onnx.json"))).map_err(|e| format!("{id}: {e}"))?;
        let config = VoiceConfig::parse(&json).map_err(|e| format!("{id}: {e}"))?;
        let model = Model::load_file(&onnx).map_err(|e| format!("{}: {e}", onnx.display()))?;
        Ok(Voice {
            id: id.to_owned(),
            config,
            model,
        })
    }

    pub fn default_scales(&self) -> Scales {
        Scales {
            noise_scale: self.config.noise_scale,
            length_scale: self.config.length_scale,
            noise_w: self.config.noise_w,
        }
    }

    /// Piper's `phoneme_ids_to_audio`: raw model output (not normalised) for one sentence's ids.
    pub fn audio(
        &self,
        ids: &[i64],
        scales: Scales,
        speaker: Option<usize>,
        pool: &Arc<rten::ThreadPool>,
    ) -> Result<Vec<f32>, String> {
        let n = ids.len();
        let ids: Vec<i32> = ids.iter().map(|&i| i32::try_from(i).unwrap_or(0)).collect();
        let input = NdTensor::from_data([1, n], ids);
        let lengths = NdTensor::from([i32::try_from(n).unwrap_or(i32::MAX)]);
        let scales = NdTensor::from([scales.noise_scale, scales.length_scale, scales.noise_w]);
        let m = &self.model;
        let id = |name: &str| m.node_id(name).map_err(|e| e.to_string());
        let mut inputs = vec![
            (id("input")?, input.into()),
            (id("input_lengths")?, lengths.into()),
            (id("scales")?, scales.into()),
        ];
        if self.config.speakers.len() > 1 {
            let sid = i32::try_from(speaker.unwrap_or(0)).unwrap_or(0);
            inputs.push((id("sid")?, NdTensor::from([sid]).into()));
        }
        let opts = RunOptions::default().with_thread_pool(Some(Arc::clone(pool)));
        let [out] = m
            .run_n(inputs, [id("output")?], Some(opts))
            .map_err(|e| e.to_string())?;
        let out: Tensor<f32> = out.try_into().map_err(|e| format!("{e:?}"))?;
        Ok(out.to_vec())
    }
}
