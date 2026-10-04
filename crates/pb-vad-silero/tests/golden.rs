//! Golden tests: speech probabilities against the ONNX model the old bot used (`silero_vad.onnx` from the silero-vad
//! 6.2.3 wheel, run frame by frame with its state and 64-sample context, stored by `tools/golden/oracle.py`, a script now in the history at 18db450).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::path::PathBuf;

use pb_models_api::{FRAME, VadModel};
use pb_vad_silero::SileroVad;
use safetensors::SafeTensors;

fn weights() -> PathBuf {
    pb_testkit::weights().join("silero-vad")
}

fn golden(name: &str) -> (Vec<f32>, Vec<f32>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.safetensors"));
    let bytes = std::fs::read(path).expect("golden file");
    let st = SafeTensors::deserialize(&bytes).expect("safetensors");
    let audio = pb_testkit::golden::pcm_f32(st.tensor("audio").expect("audio").data());
    let probs = pb_testkit::golden::f32s(st.tensor("probs").expect("probs").data());
    (audio, probs)
}

fn frames(audio: &[f32]) -> Vec<[f32; FRAME]> {
    audio.as_chunks::<FRAME>().0.to_vec()
}

#[test]
#[ignore = "needs the Silero weights (PB_WEIGHTS); run with --ignored"]
fn matches_the_onnx_model_frame_by_frame() {
    let mut vad = SileroVad::load(&weights()).expect("loads");
    for name in ["jfk", "noise", "dialogue"] {
        let (audio, want) = golden(name);
        let mut state = vad.new_state();
        let started = std::time::Instant::now();
        let got: Vec<f32> = frames(&audio)
            .iter()
            .map(|f| vad.step(std::slice::from_ref(f), &mut [&mut state])[0])
            .collect();
        let per_frame = started.elapsed() / got.len() as u32;
        assert_eq!(got.len(), want.len());
        let worst = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        eprintln!(
            "{name:<9} {} frames, |Δp| ≤ {worst:.2e}, {per_frame:?} per frame",
            got.len()
        );
        assert!(worst <= 1e-4, "{name}: probabilities differ by {worst}");
    }
}

#[test]
#[ignore = "needs the Silero weights (PB_WEIGHTS); run with --ignored"]
fn batches_many_streams() {
    let mut vad = SileroVad::load(&weights()).expect("loads");
    let (audio, want) = golden("jfk");
    let frames = frames(&audio);
    let streams = 30;
    let mut states: Vec<_> = (0..streams).map(|_| vad.new_state()).collect();
    let started = std::time::Instant::now();
    for (k, f) in frames.iter().enumerate() {
        let batch = vec![*f; streams];
        let mut refs: Vec<_> = states.iter_mut().collect();
        let p = vad.step(&batch, &mut refs);
        for q in &p {
            assert!((q - want[k]).abs() <= 1e-4, "frame {k}: {q} vs {}", want[k]);
        }
    }
    let per = started.elapsed() / (frames.len() * streams) as u32;
    eprintln!("{streams} streams batched: {per:?} per stream-frame");
}

#[test]
#[ignore = "needs the Silero weights (PB_WEIGHTS); run with --ignored"]
fn honours_the_vad_contract() {
    let mut vad = SileroVad::load(&weights()).expect("loads");
    let (audio, _) = golden("jfk");
    pb_models_api::contract::vad(&mut vad, &audio[..FRAME * 40]);
}
