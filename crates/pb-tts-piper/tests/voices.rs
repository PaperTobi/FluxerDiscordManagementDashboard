//! The voices on rten against Piper 1.8.0 on onnxruntime: with both noise scales at 0 the model is deterministic, so
//! the audio must match; plus speed and the engine contract.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::path::PathBuf;
use std::sync::Arc;

use pb_models_api::{SpeakOpts, TtsEngine};
use pb_tts_piper::{PiperEngine, Scales, Voice};
use safetensors::SafeTensors;

fn voices_dir() -> PathBuf {
    pb_testkit::weights().join("voices")
}

const VOICES: [&str; 4] = [
    "de_DE-thorsten-high",
    "de_DE-thorsten-medium",
    "en_US-lessac-high",
    "en_US-lessac-medium",
];

#[test]
#[ignore = "needs the voices (PB_WEIGHTS); run with --ignored"]
fn noise_free_audio_matches_piper() {
    let pool = Arc::new(rten::ThreadPool::with_num_threads(4));
    for id in VOICES {
        let voice = Voice::load(&voices_dir().join(id), id).expect("loads");
        let bytes = std::fs::read(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/golden/audio_{id}.safetensors")),
        )
        .expect("golden");
        let st = SafeTensors::deserialize(&bytes).expect("safetensors");
        for i in 0.. {
            let Ok(ids) = st.tensor(&format!("s{i}_ids")) else {
                break;
            };
            let ids: Vec<i64> = pb_testkit::golden::i64s(ids.data());
            let want: Vec<f32> = pb_testkit::golden::f32s(st.tensor(&format!("s{i}_audio")).expect("audio").data());
            let quiet = Scales {
                noise_scale: 0.0,
                length_scale: 1.0,
                noise_w: 0.0,
            };
            let got = voice.audio(&ids, quiet, None, &pool).expect("synthesizes");
            assert_eq!(got.len(), want.len(), "{id} s{i}: same length");
            let worst = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            let peak = want.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            eprintln!("{id} s{i}: {} samples, |Δ| ≤ {worst:.2e} (peak {peak:.3})", got.len());
            assert!(worst <= 1e-3 * peak.max(1e-3), "{id} s{i}: audio differs by {worst}");
        }
    }
}

#[test]
#[ignore = "needs the voices (PB_WEIGHTS); run with --ignored"]
fn speaks_faster_than_real_time_and_honours_the_contract() {
    let threads: usize = std::env::var("PB_TTS_THREADS")
        .ok()
        .and_then(|t| t.parse().ok())
        .unwrap_or(2);
    let mut engine = PiperEngine::new(
        std::path::Path::new(pb_espeak::BUILD_DATA_DIR),
        &[voices_dir()],
        threads,
    )
    .expect("engine");
    for (id, text) in [
        ("de_DE-thorsten-high", "Hey Richard, achte auf deine Wortwahl!"),
        ("de_DE-thorsten-medium", "Hey Richard, achte auf deine Wortwahl!"),
        ("en_US-lessac-high", "Hey Richard, watch your language!"),
    ] {
        let _ = engine.synthesize(id, text, &SpeakOpts::default()).expect("warm-up");
        let t = std::time::Instant::now();
        let speech = engine.synthesize(id, text, &SpeakOpts::default()).expect("speaks");
        let took = t.elapsed().as_secs_f32();
        let seconds = speech.samples.len() as f32 / speech.sample_rate as f32;
        eprintln!(
            "{id}: {seconds:.2} s of speech in {took:.2} s (real-time factor {:.2}, {threads} threads)",
            took / seconds
        );
        assert!(speech.unknown_phonemes.is_empty(), "{:?}", speech.unknown_phonemes);
        pb_models_api::contract::tts(&mut engine, id, text);
    }
}
