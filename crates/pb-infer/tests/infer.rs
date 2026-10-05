#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pb_infer::{InferError, Inference, Models, Priority, SpeakPriority, TtsFactory};
use pb_models_api::{
    Classifier, ClassifierInfo, FRAME, ModelError, RawScores, SpeakOpts, Speech, TtsEngine, TtsError, TtsInfo, VadInfo,
    VadModel, VadState, VoiceInfo,
};

/// Scores the mean of the clip; records the order of calls.
struct FakeClassifier {
    info: ClassifierInfo,
    log: Arc<Mutex<Vec<String>>>,
    delay: Duration,
}

impl Classifier for FakeClassifier {
    fn info(&self) -> &ClassifierInfo {
        &self.info
    }
    fn classify(&mut self, pcm: &[f32]) -> Result<RawScores, ModelError> {
        if pcm.len() < self.info.min_samples {
            return Err(ModelError::TooShort {
                samples: pcm.len(),
                min: self.info.min_samples,
            });
        }
        if pcm.len() > self.info.max_samples {
            return Err(ModelError::TooLong {
                samples: pcm.len(),
                max: self.info.max_samples,
            });
        }
        std::thread::sleep(self.delay);
        let mean = pcm.iter().sum::<f32>() / pcm.len() as f32;
        self.log.lock().unwrap().push(format!("{mean:.1}"));
        let mut languages = [0.0; 30];
        languages[if mean > 0.5 { 4 } else { 6 }] = 1.0;
        Ok(RawScores {
            labels: [mean; 8],
            languages,
        })
    }
    fn set_threads(&mut self, _: NonZeroUsize) -> Result<(), ModelError> {
        self.log.lock().unwrap().push("threads".into());
        Ok(())
    }
}

/// Probability = the frame's first sample; the state counts frames (to check state is per stream).
struct FakeVad(VadInfo);

impl VadModel for FakeVad {
    fn info(&self) -> &VadInfo {
        &self.0
    }
    fn new_state(&self) -> VadState {
        VadState(vec![0.0])
    }
    fn step(&mut self, frames: &[[f32; FRAME]], states: &mut [&mut VadState]) -> Vec<f32> {
        frames
            .iter()
            .zip(states.iter_mut())
            .map(|(f, s)| {
                s.0[0] += 1.0;
                if f[0] < 0.0 { f32::NAN } else { f[0] + s.0[0] * 0.0 }
            })
            .collect()
    }
}

/// A speech model with one voice `v` that says 0.1 s (`slow`: 0.2 s) per letter.
struct FakeTts {
    model: &'static str,
    slow: bool,
}

const FAKE: FakeTts = FakeTts {
    model: "fake",
    slow: false,
};

impl TtsEngine for FakeTts {
    fn info(&self) -> TtsInfo {
        TtsInfo {
            model: self.model.into(),
            cloning: false,
        }
    }

    fn voices(&self) -> Vec<VoiceInfo> {
        vec![VoiceInfo {
            id: "v".into(),
            model: self.model.into(),
            language: "en".into(),
            languages: vec!["en".into()],
            speakers: vec![],
            sample_rate: 22_050,
            quality: "x".into(),
        }]
    }
    fn synthesize(&mut self, voice: &str, text: &str, _: &SpeakOpts) -> Result<Speech, TtsError> {
        if voice != "v" {
            return Err(TtsError::NoVoice(voice.into()));
        }
        let n = 2205 * text.len() * if self.slow { 2 } else { 1 };
        Ok(Speech {
            samples: (0..n).map(|i| (i as f32 * 0.05).sin() * 0.5).collect(),
            sample_rate: 22_050,
            unknown_phonemes: vec![],
        })
    }
}

fn start(delay: Duration) -> (Inference, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let classifier = FakeClassifier {
        info: ClassifierInfo {
            model: "fake".into(),
            min_samples: 480,
            max_samples: 480_000,
            device: "cpu".into(),
        },
        log: log.clone(),
        delay,
    };
    let models = Models {
        vad: Box::new(FakeVad(VadInfo {
            model: "fake".into(),
            context: 0,
        })),
        classifier: Box::new(classifier),
        tts: vec![Box::new(|_| Ok(Box::new(FAKE) as Box<dyn TtsEngine>))],
        tts_threads: 1,
    };
    (Inference::start(models).unwrap(), log)
}

#[tokio::test(flavor = "multi_thread")]
async fn live_jobs_go_first() {
    let (inf, log) = start(Duration::from_millis(30));
    let clip = |v: f32| -> Arc<[f32]> { vec![v; 16_000].into() };
    // The first job occupies the worker; the rest queue up and are taken by priority.
    let first = tokio::spawn({
        let inf = inf.clone();
        async move { inf.classify(clip(0.1), Priority::Import).await }
    });
    tokio::time::sleep(Duration::from_millis(10)).await;
    let mut jobs = Vec::new();
    for (v, p) in [(0.2, Priority::Import), (0.3, Priority::Check), (0.4, Priority::Live)] {
        let inf = inf.clone();
        jobs.push(tokio::spawn(async move { inf.classify(clip(v), p).await }));
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    first.await.unwrap().unwrap();
    for j in jobs {
        j.await.unwrap().unwrap();
    }
    assert_eq!(*log.lock().unwrap(), vec!["0.1", "0.4", "0.3", "0.2"]);
    let status = inf.status();
    assert_eq!((status.classify.done, status.classify.waiting), (4, 0));
    inf.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn long_speech_is_scored_in_windows() {
    let (inf, log) = start(Duration::ZERO);
    // 40 s: windows [0, 30 s) (mean 0.4) and [25 s, 40 s) (mean 0.8); the second half is louder.
    let mut pcm = vec![0.2f32; 640_000];
    for x in &mut pcm[320_000..] {
        *x = 0.8;
    }
    let s = inf.classify(pcm.into(), Priority::Live).await.unwrap();
    assert_eq!(s.windows, 2);
    assert_eq!(log.lock().unwrap().len(), 2);
    // Labels: the highest window; languages: weighted by window length.
    assert!((s.raw.labels[0] - 0.8).abs() < 5e-3, "{:?}", s.raw.labels);
    assert_eq!(*log.lock().unwrap(), vec!["0.4", "0.8"]);
    assert!((s.raw.languages.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    let short = inf.classify(vec![0.0f32; 10].into(), Priority::Live).await;
    assert!(matches!(short, Err(InferError::Model(ModelError::TooShort { .. }))));
    inf.set_classifier_threads(NonZeroUsize::new(2).unwrap());
    inf.classify(vec![0.0f32; 1000].into(), Priority::Live).await.unwrap();
    assert!(log.lock().unwrap().contains(&"threads".to_string()));
    inf.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn vad_streams_are_batched_but_separate() {
    let (inf, _) = start(Duration::ZERO);
    let mut a = inf.vad_stream();
    let mut b = inf.vad_stream();
    let frame = |v: f32| [v; FRAME];
    let (pa, pb) = tokio::join!(
        a.step(vec![frame(0.1), frame(0.2), frame(0.3)]),
        b.step(vec![frame(0.9)])
    );
    assert_eq!(pa.unwrap(), vec![0.1, 0.2, 0.3]);
    assert_eq!(pb.unwrap(), vec![0.9]);
    // A frame the model cannot answer is answered by the energy gate.
    let loud: [f32; FRAME] = std::array::from_fn(|i| if i == 0 { -0.5 } else { ((i as f32) * 0.3).sin() * 0.5 });
    let p = a.step(vec![loud]).await.unwrap();
    assert!(p[0].is_finite() && (0.0..=1.0).contains(&p[0]));
    assert_eq!(inf.status().vad_fallbacks, 1);
    assert_eq!(inf.status().vad_streams, 2);
    drop(b);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(inf.status().vad_streams, 1);
    inf.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn speech_comes_out_at_48_khz() {
    let (inf, _) = start(Duration::ZERO);
    let voices = inf.reload_tts(2).await.unwrap();
    assert_eq!(voices.len(), 1);
    let s = inf
        .speak("v", "hi", SpeakOpts::default(), SpeakPriority::Live)
        .await
        .unwrap();
    let secs = s.samples.len() as f32 / 48_000.0;
    assert!((secs - 0.2).abs() < 0.01, "{secs}");
    assert!(matches!(
        inf.speak("nope", "hi", SpeakOpts::default(), SpeakPriority::Preview)
            .await,
        Err(InferError::Tts(TtsError::NoVoice(_)))
    ));
    inf.shutdown();
    assert!(matches!(
        inf.classify(vec![0.0; 1000].into(), Priority::Live).await,
        Err(InferError::Stopped)
    ));
}

#[tokio::test(flavor = "multi_thread")]
async fn each_voice_goes_to_its_own_model() {
    let engine = |model: &'static str, slow: bool| -> TtsFactory {
        Box::new(move |_| Ok(Box::new(FakeTts { model, slow }) as Box<dyn TtsEngine>))
    };
    let models = Models {
        vad: Box::new(FakeVad(VadInfo {
            model: "fake".into(),
            context: 0,
        })),
        classifier: Box::new(FakeClassifier {
            info: ClassifierInfo {
                model: "fake".into(),
                min_samples: 480,
                max_samples: 480_000,
                device: "cpu".into(),
            },
            log: Arc::default(),
            delay: Duration::ZERO,
        }),
        tts: vec![engine("fast", false), engine("slow", true)],
        tts_threads: 1,
    };
    let inf = Inference::start(models).unwrap();
    let voices = inf.reload_tts(1).await.unwrap();
    let ids: Vec<String> = voices.iter().map(VoiceInfo::full_id).collect();
    assert_eq!(ids, ["fast:v", "slow:v"]);
    assert_eq!(inf.status().voices, 2);
    let secs = |voice: &'static str| {
        let inf = inf.clone();
        async move {
            let s = inf
                .speak(voice, "hi", SpeakOpts::default(), SpeakPriority::Live)
                .await?;
            Ok::<f32, InferError>(s.samples.len() as f32 / 48_000.0)
        }
    };
    // A bare id is the first model that has it; `<model>:<id>` picks the model.
    assert!((secs("v").await.unwrap() - 0.2).abs() < 0.01);
    assert!((secs("fast:v").await.unwrap() - 0.2).abs() < 0.01);
    assert!((secs("slow:v").await.unwrap() - 0.4).abs() < 0.01);
    assert!(matches!(
        secs("other:v").await,
        Err(InferError::Tts(TtsError::NoVoice(_)))
    ));
    inf.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn voices_made_from_samples_stay_across_reloads() {
    let models = Models {
        vad: Box::new(FakeVad(VadInfo {
            model: "fake".into(),
            context: 0,
        })),
        classifier: Box::new(FakeClassifier {
            info: ClassifierInfo {
                model: "fake".into(),
                min_samples: 480,
                max_samples: 480_000,
                device: "cpu".into(),
            },
            log: Arc::default(),
            delay: Duration::ZERO,
        }),
        tts: vec![
            Box::new(|_| Ok(Box::new(FAKE) as Box<dyn TtsEngine>)),
            Box::new(|_| Ok(Box::new(pb_testkit::models::BeepTts::default()) as Box<dyn TtsEngine>)),
        ],
        tts_threads: 1,
    };
    let inf = Inference::start(models).unwrap();
    inf.reload_tts(1).await.unwrap();
    let cloning: Vec<(String, bool)> = inf.speech_models().into_iter().map(|m| (m.model, m.cloning)).collect();
    assert_eq!(cloning, [("fake".to_owned(), false), ("beep".to_owned(), true)]);
    let tone: Vec<f32> = pb_testkit::models::tone(300.0, 1.0, 16_000, 0.3)
        .iter()
        .map(|&s| f32::from(s) / 32768.0)
        .collect();
    assert!(matches!(
        inf.clone_voice("fake", tone.clone(), 16_000, None).await,
        Err(InferError::Tts(TtsError::NoCloning))
    ));
    assert!(matches!(
        inf.clone_voice("omni", tone.clone(), 16_000, None).await,
        Err(InferError::NoModel(_))
    ));
    let made = inf
        .clone_voice("beep", tone, 16_000, Some("Hello.".into()))
        .await
        .unwrap();
    let info = inf.add_voice("beep", "low", made).await.unwrap();
    assert_eq!(info.full_id(), "beep:low");
    // Kept when the models are started again (new voices installed, another thread count).
    inf.reload_tts(2).await.unwrap();
    assert!(inf.voices().iter().any(|v| v.full_id() == "beep:low"));
    inf.speak("beep:low", "hi", SpeakOpts::default(), SpeakPriority::Preview)
        .await
        .unwrap();
    inf.remove_voice("beep", "low").await.unwrap();
    assert!(!inf.voices().iter().any(|v| v.full_id() == "beep:low"));
    inf.reload_tts(1).await.unwrap();
    assert!(!inf.voices().iter().any(|v| v.full_id() == "beep:low"));
    inf.shutdown();
}
