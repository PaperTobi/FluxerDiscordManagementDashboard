//! Stand-in models for tests that run the whole bot without model weights. Speech is a plain tone and its pitch says
//! what was said: [`ToneVad`] hears any tone as speech, [`ToneClassifier`] scores a low tone (about 440 Hz) as harmless
//! and a high one (about 880 Hz) as profanity, and [`BeepTts`] answers every text with a beep as long as the text.
//! [`tone`] makes the audio.

use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use pb_domain::{ClfLang, Label};
use pb_models_api::{
    Classifier, ClassifierInfo, FRAME, ModelError, RawScores, SpeakOpts, Speech, TtsEngine, TtsError, VadInfo,
    VadModel, VadState, VoiceInfo,
};

/// The pitch of harmless speech.
pub const BENIGN_HZ: f32 = 440.0;
/// The pitch of profanity.
pub const PROFANE_HZ: f32 = 880.0;
/// Pitches from here up are profanity.
const PROFANE_FROM_HZ: f32 = 660.0;

/// A sine tone of `freq` Hz, `secs` long at `rate`, with peak `amp` (0 to 1), as 16-bit samples.
pub fn tone(freq: f32, secs: f32, rate: u32, amp: f32) -> Vec<i16> {
    let n = (secs * rate as f32).round() as usize;
    let step = std::f32::consts::TAU * freq / rate as f32;
    (0..n)
        .map(|i| ((i as f32 * step).sin() * amp * 32767.0).round() as i16)
        .collect()
}

/// The pitch of the audible part of a clip, from its zero crossings (0 for silence).
fn pitch(pcm: &[f32], rate: u32) -> f32 {
    let audible: Vec<f32> = pcm.iter().copied().filter(|x| x.abs() > 1e-3).collect();
    if audible.len() < 2 {
        return 0.0;
    }
    let crossings = audible.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
    crossings as f32 * rate as f32 / (2.0 * audible.len() as f32)
}

/// Voice activity: a frame louder than an RMS of 0.01 (-40 dBFS) is speech.
#[derive(Debug)]
pub struct ToneVad {
    info: VadInfo,
}

impl Default for ToneVad {
    fn default() -> Self {
        ToneVad {
            info: VadInfo {
                model: "tone-vad".into(),
                context: 0,
            },
        }
    }
}

impl VadModel for ToneVad {
    fn info(&self) -> &VadInfo {
        &self.info
    }

    fn new_state(&self) -> VadState {
        VadState(Vec::new())
    }

    fn step(&mut self, frames: &[[f32; FRAME]], _states: &mut [&mut VadState]) -> Vec<f32> {
        frames
            .iter()
            .map(|f| {
                let rms = (f.iter().map(|x| x * x).sum::<f32>() / FRAME as f32).sqrt();
                if rms > 0.01 { 1.0 } else { 0.0 }
            })
            .collect()
    }
}

/// Holds classifications back while it is closed (a busy or slow model); clones share it.
#[derive(Debug, Clone, Default)]
pub struct Gate {
    /// Whether it is closed, and the waiting classifications.
    closed: Arc<(Mutex<bool>, Condvar)>,
}

impl Gate {
    /// A gate that holds every classification until [`Gate::open`].
    pub fn closed() -> Gate {
        let gate = Gate::default();
        gate.close();
        gate
    }

    pub fn close(&self) {
        *self.lock() = true;
    }

    /// Lets the waiting classifications and every later one through.
    pub fn open(&self) {
        *self.lock() = false;
        self.closed.1.notify_all();
    }

    fn lock(&self) -> MutexGuard<'_, bool> {
        self.closed.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits while the gate is closed.
    fn pass(&self) {
        let mut closed = self.lock();
        while *closed {
            closed = self.closed.1.wait(closed).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// The voice-safety classifier for tones: [`PROFANE_HZ`] scores as clear profanity, [`BENIGN_HZ`] as harmless speech,
/// both with scores shaped like the real model's (independent probabilities per type, a language distribution summing
/// to 1).
#[derive(Debug)]
pub struct ToneClassifier {
    info: ClassifierInfo,
    language: ClfLang,
    gate: Option<Gate>,
    calls: Arc<AtomicUsize>,
}

impl Default for ToneClassifier {
    fn default() -> Self {
        ToneClassifier {
            info: ClassifierInfo {
                model: "tone-classifier".into(),
                min_samples: 480,
                max_samples: 480_000,
                device: "none".into(),
            },
            language: ClfLang::En,
            gate: None,
            calls: Arc::default(),
        }
    }
}

impl ToneClassifier {
    /// Hears every clip in `language` (English by default).
    pub fn language(mut self, language: ClfLang) -> Self {
        self.language = language;
        self
    }

    /// Waits at `gate` before each clip.
    pub fn gated(mut self, gate: Gate) -> Self {
        self.gate = Some(gate);
        self
    }

    /// How many clips it was given so far (counted on arrival, before the gate).
    pub fn calls(&self) -> Arc<AtomicUsize> {
        self.calls.clone()
    }

    fn scores(&self, pitch: f32) -> RawScores {
        let mut labels = [0.0f32; 8];
        let profane = pitch >= PROFANE_FROM_HZ;
        for label in Label::ALL {
            labels[label.index()] = match label {
                Label::Profanity if profane => 0.97,
                Label::Harassment if profane => 0.18,
                Label::Profanity => 0.04,
                Label::DisruptiveAudio => 0.05,
                _ => 0.02,
            };
        }
        let mut languages = [0.1 / (ClfLang::ALL.len() - 1) as f32; 30];
        languages[self.language.index()] = 0.9;
        RawScores { labels, languages }
    }
}

impl Classifier for ToneClassifier {
    fn info(&self) -> &ClassifierInfo {
        &self.info
    }

    fn classify(&mut self, pcm16k: &[f32]) -> Result<RawScores, ModelError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.pass();
        }
        if pcm16k.len() < self.info.min_samples {
            return Err(ModelError::TooShort {
                samples: pcm16k.len(),
                min: self.info.min_samples,
            });
        }
        if pcm16k.len() > self.info.max_samples {
            return Err(ModelError::TooLong {
                samples: pcm16k.len(),
                max: self.info.max_samples,
            });
        }
        Ok(self.scores(pitch(pcm16k, pb_domain::LISTEN_RATE)))
    }

    fn set_threads(&mut self, _threads: NonZeroUsize) -> Result<(), ModelError> {
        Ok(())
    }
}

/// One request to [`BeepTts`].
#[derive(Debug, Clone, PartialEq)]
pub struct Spoken {
    pub voice: String,
    pub text: String,
    pub rate: f32,
}

/// Text-to-speech with an English and a German voice. Each says any text as a beep of 20 ms per character (at rate 1),
/// at 660 Hz in English and 520 Hz in German. Every request is logged; clones share the log.
#[derive(Debug, Clone, Default)]
pub struct BeepTts {
    log: Arc<Mutex<Vec<Spoken>>>,
}

impl BeepTts {
    pub const EN: &str = "en_US-beep-medium";
    pub const DE: &str = "de_DE-beep-medium";
    const RATE: u32 = 22_050;
    const MS_PER_CHAR: f32 = 20.0;

    /// Every request so far, in order.
    pub fn log(&self) -> Vec<Spoken> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl TtsEngine for BeepTts {
    fn voices(&self) -> Vec<VoiceInfo> {
        [(BeepTts::EN, "en_US"), (BeepTts::DE, "de_DE")]
            .into_iter()
            .map(|(id, language)| VoiceInfo {
                id: id.into(),
                language: language.into(),
                speakers: Vec::new(),
                sample_rate: BeepTts::RATE,
                quality: "medium".into(),
            })
            .collect()
    }

    fn synthesize(&mut self, voice: &str, text: &str, opts: &SpeakOpts) -> Result<Speech, TtsError> {
        self.log.lock().unwrap_or_else(PoisonError::into_inner).push(Spoken {
            voice: voice.into(),
            text: text.into(),
            rate: opts.rate,
        });
        let freq = match voice {
            BeepTts::EN => 660.0,
            BeepTts::DE => 520.0,
            other => return Err(TtsError::NoVoice(other.into())),
        };
        if text.trim().is_empty() {
            return Err(TtsError::Empty);
        }
        let secs = text.chars().count() as f32 * BeepTts::MS_PER_CHAR / 1000.0 / opts.rate;
        Ok(Speech {
            samples: tone(freq, secs, BeepTts::RATE, 0.5)
                .into_iter()
                .map(|s| f32::from(s) / 32768.0)
                .collect(),
            sample_rate: BeepTts::RATE,
            unknown_phonemes: Vec::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floats(pcm: &[i16]) -> Vec<f32> {
        pcm.iter().map(|&s| f32::from(s) / 32768.0).collect()
    }

    fn speech(freq: f32) -> Vec<f32> {
        floats(&tone(freq, 1.0, pb_domain::LISTEN_RATE, 0.3))
    }

    #[test]
    fn the_stand_ins_keep_the_model_contracts() {
        pb_models_api::contract::vad(&mut ToneVad::default(), &speech(BENIGN_HZ));
        pb_models_api::contract::classifier(&mut ToneClassifier::default(), &speech(PROFANE_HZ));
        pb_models_api::contract::tts(&mut BeepTts::default(), BeepTts::EN, "Please keep it clean.");
        pb_models_api::contract::tts(&mut BeepTts::default(), BeepTts::DE, "Bitte nicht fluchen.");
    }

    #[test]
    fn pitch_decides_what_was_said() {
        let mut clf = ToneClassifier::default().language(ClfLang::De);
        // A pause before the speech (as the segmenter keeps it) does not change the pitch.
        let mut quiet_then_high = vec![0.0; 4800];
        quiet_then_high.extend(speech(PROFANE_HZ));
        let high = clf.classify(&quiet_then_high).unwrap();
        let low = clf.classify(&speech(BENIGN_HZ)).unwrap();
        assert!(high.label(Label::Profanity) > 0.9 && low.label(Label::Profanity) < 0.1);
        assert!(low.labels.iter().all(|p| *p < 0.5), "{low:?}");
        assert_eq!(high.top_language(), ClfLang::De);
        assert_eq!(clf.calls().load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_closed_gate_holds_the_classification_back() {
        let gate = Gate::closed();
        let mut clf = ToneClassifier::default().gated(gate.clone());
        let calls = clf.calls();
        let worker = std::thread::spawn(move || clf.classify(&speech(BENIGN_HZ)));
        while calls.load(Ordering::SeqCst) == 0 {
            std::thread::yield_now();
        }
        assert!(!worker.is_finished());
        gate.open();
        assert!(worker.join().unwrap().is_ok());
    }

    #[test]
    fn speech_length_follows_the_text_and_requests_are_logged() {
        let mut tts = BeepTts::default();
        let log = tts.clone();
        let short = tts.synthesize(BeepTts::EN, "Hi.", &SpeakOpts::default()).unwrap();
        let long = tts.synthesize(BeepTts::EN, "Hi there.", &SpeakOpts::default()).unwrap();
        assert_eq!(short.samples.len() * 3, long.samples.len());
        assert_eq!(
            log.log().iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            ["Hi.", "Hi there."]
        );
    }
}
