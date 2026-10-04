//! Piper text-to-speech: espeak-ng phonemes (the C library at Piper's commit, see `pb-espeak`) → Piper phoneme ids →
//! the voice's VITS model on rten (pure Rust). Faithful to piper-tts 1.8.0, which the old bot used: same phonemes and
//! ids (golden tests), the model's own noise/length scales, each sentence peak-normalised and clipped as `voice.py`
//! does, sentences joined with a configurable pause.

pub mod phonemes;
pub mod voice;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pb_espeak::Espeak;
use pb_models_api::{SpeakOpts, Speech, TtsEngine, TtsError, VoiceInfo};

pub use voice::{Scales, Voice, VoiceConfig};

/// Every Piper voice found in a set of directories (`<dir>/<id>/<id>.onnx` + `.onnx.json`), loaded on first use.
pub struct PiperEngine {
    espeak: Espeak,
    found: BTreeMap<String, (PathBuf, VoiceConfig)>,
    loaded: BTreeMap<String, Voice>,
    pool: Arc<rten::ThreadPool>,
}

impl std::fmt::Debug for PiperEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PiperEngine")
            .field("voices", &self.found.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl PiperEngine {
    /// `espeak_data`: the directory containing `espeak-ng-data`; `voice_dirs`: searched in order, a later directory
    /// wins for the same voice id; `threads`: CPU threads for the voice model.
    pub fn new(espeak_data: &Path, voice_dirs: &[PathBuf], threads: usize) -> Result<Self, TtsError> {
        let espeak = Espeak::open(espeak_data).map_err(|e| TtsError::Load(e.to_string()))?;
        let mut engine = PiperEngine {
            espeak,
            found: BTreeMap::new(),
            loaded: BTreeMap::new(),
            pool: Arc::new(rten::ThreadPool::with_num_threads(threads.max(1))),
        };
        engine.rescan(voice_dirs);
        Ok(engine)
    }

    /// Looks for voices again (after one was installed or removed); loaded voices that are gone are dropped.
    pub fn rescan(&mut self, voice_dirs: &[PathBuf]) {
        self.found.clear();
        for dir in voice_dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(id) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
                    continue;
                };
                let json = path.join(format!("{id}.onnx.json"));
                if !path.join(format!("{id}.onnx")).is_file() {
                    continue;
                }
                if let Ok(config) = std::fs::read_to_string(&json)
                    .map_err(|e| e.to_string())
                    .and_then(|j| VoiceConfig::parse(&j))
                {
                    self.found.insert(id, (path, config));
                }
            }
        }
        self.loaded.retain(|id, _| self.found.contains_key(id));
    }

    fn voice(&mut self, id: &str) -> Result<&Voice, TtsError> {
        if !self.loaded.contains_key(id) {
            let (dir, _) = self.found.get(id).ok_or_else(|| TtsError::NoVoice(id.to_owned()))?;
            let voice = Voice::load(dir, id).map_err(TtsError::Load)?;
            self.loaded.insert(id.to_owned(), voice);
        }
        Ok(&self.loaded[id])
    }
}

impl TtsEngine for PiperEngine {
    fn voices(&self) -> Vec<VoiceInfo> {
        self.found
            .iter()
            .map(|(id, (_, c))| VoiceInfo {
                id: id.clone(),
                model: "piper".into(),
                language: c.language.clone(),
                languages: vec![c.language.clone()],
                speakers: c.speakers.clone(),
                sample_rate: c.sample_rate,
                quality: c.quality.clone(),
            })
            .collect()
    }

    fn synthesize(&mut self, voice: &str, text: &str, opts: &SpeakOpts) -> Result<Speech, TtsError> {
        if !self.found.contains_key(voice) {
            return Err(TtsError::NoVoice(voice.to_owned()));
        }
        if text.trim().is_empty() {
            return Err(TtsError::Empty);
        }
        let (espeak_voice, clusters) = {
            let c = &self.found[voice].1;
            (c.espeak_voice.clone(), c.vowel_clusters.clone())
        };
        let sentences = phonemes::phonemize(&mut self.espeak, &espeak_voice, text, clusters.as_ref())
            .map_err(|e| TtsError::Phonemes(e.to_string()))?;
        let pool = Arc::clone(&self.pool);
        let v = self.voice(voice)?;
        let rate = if opts.rate.is_finite() && opts.rate > 0.0 {
            opts.rate
        } else {
            1.0
        };
        let mut scales = v.default_scales();
        scales.length_scale /= rate;
        let gap = (u64::from(v.config.sample_rate) * u64::from(opts.sentence_gap_ms) / 1000) as usize;
        let mut samples = Vec::new();
        let mut unknown = Vec::new();
        for sentence in sentences.iter().filter(|s| !s.is_empty()) {
            let (ids, missing) = phonemes::to_ids(sentence, &v.config.phoneme_id_map);
            unknown.extend(missing);
            let mut audio = v.audio(&ids, scales, opts.speaker, &pool).map_err(TtsError::Failed)?;
            // voice.py: normalize_audio (peak to 1.0, silence stays silent), volume 1.0, clip to [-1, 1].
            let peak = audio.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            if peak < 1e-8 {
                audio.iter_mut().for_each(|x| *x = 0.0);
            } else {
                audio.iter_mut().for_each(|x| *x = (*x / peak).clamp(-1.0, 1.0));
            }
            if !samples.is_empty() {
                samples.resize(samples.len() + gap, 0.0);
            }
            samples.extend(audio);
        }
        if samples.is_empty() {
            return Err(TtsError::Empty);
        }
        Ok(Speech {
            samples,
            sample_rate: v.config.sample_rate,
            unknown_phonemes: unknown,
        })
    }
}
