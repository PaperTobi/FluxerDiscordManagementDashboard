//! The real models behind the inference handle. Needs the weights (`PB_WEIGHTS`, default `target/weights`).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::num::NonZeroUsize;
use std::path::PathBuf;

use pb_domain::{ClfLang, Label};
use pb_infer::{Inference, Models, Priority, SpeakPriority};
use pb_models_api::{FRAME, SpeakOpts, TtsEngine};

fn weights() -> PathBuf {
    pb_testkit::weights()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the model weights (PB_WEIGHTS); run with --ignored"]
async fn the_real_models_answer_through_the_handle() {
    let w = weights();
    let vad = pb_vad_silero::SileroVad::load(&w.join("silero-vad")).unwrap();
    let clf = pb_classifier_roblox::RobloxClassifier::load_cpu(
        &w.join("roblox-voice-safety-v3"),
        NonZeroUsize::new(4).unwrap(),
    )
    .unwrap();
    let voices = w.join("voices");
    let inf = Inference::start(Models {
        vad: Box::new(vad),
        classifier: Box::new(clf),
        tts: vec![Box::new(move |threads| {
            pb_tts_piper::PiperEngine::new(
                std::path::Path::new(pb_espeak::BUILD_DATA_DIR),
                std::slice::from_ref(&voices),
                threads,
            )
            .map(|e| Box::new(e) as Box<dyn TtsEngine>)
        })],
        tts_threads: 2,
    })
    .unwrap();
    let (pcm, rate) = pb_testkit::audio::read_wav(&pb_testkit::fixture("jfk.wav")).unwrap();
    assert_eq!(rate, 16_000);
    let x = pb_audio::from_i16(&pcm);

    let mut stream = inf.vad_stream();
    let frames: Vec<[f32; FRAME]> = x.as_chunks::<FRAME>().0.to_vec();
    let probs = stream.step(frames).await.unwrap();
    let voiced = probs.iter().filter(|p| **p >= 0.5).count() as f32 / probs.len() as f32;
    assert!(voiced > 0.6, "JFK speaks most of the clip: {voiced}");

    let s = inf.classify(x.into(), Priority::Live).await.unwrap();
    assert!(s.raw.label(Label::Profanity) < 0.5, "{:?}", s.raw);
    assert_eq!(s.raw.top_language(), ClfLang::En);
    println!("classify: {} ms, VAD frames {}", s.infer_ms, inf.status().vad_frames);

    let speech = inf
        .speak(
            "de_DE-thorsten-medium",
            "Hey Richard, achte auf deine Wortwahl!",
            SpeakOpts::default(),
            SpeakPriority::Live,
        )
        .await
        .unwrap();
    let secs = speech.samples.len() as f32 / 48_000.0;
    assert!(secs > 1.0 && secs < 6.0, "{secs}");
    assert!(inf.voices().len() >= 4);
    inf.shutdown();
}
