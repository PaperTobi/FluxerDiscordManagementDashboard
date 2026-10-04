//! Golden tests: the Burn model against the PyTorch reference (`tools/golden/oracle.py`, a script now in the history at 18db450, ran Roblox's `inference.py`
//! the way the old bot did and stored inputs, intermediate tensors and outputs in `tests/golden/`).
//! Needs the model weights: `PB_WEIGHTS` (directory containing `roblox-voice-safety-v3/`).

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use std::num::NonZeroUsize;
use std::path::PathBuf;

use burn::tensor::Tensor;
use burn::tensor::activation::{sigmoid, softmax};
use burn::tensor::backend::Backend;
use pb_classifier_roblox::{Cpu, RobloxClassifier};
use pb_models_api::Classifier;
use safetensors::SafeTensors;

fn model_dir() -> PathBuf {
    pb_testkit::weights().join("roblox-voice-safety-v3")
}

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(name)
}

fn floats(st: &SafeTensors<'_>, name: &str) -> (Vec<f32>, Vec<usize>) {
    let t = st.tensor(name).unwrap_or_else(|e| panic!("{name}: {e}"));
    (pb_testkit::golden::f32s(t.data()), t.shape().to_vec())
}

fn audio(st: &SafeTensors<'_>) -> Vec<f32> {
    pb_testkit::golden::pcm_f32(st.tensor("audio").expect("audio").data())
}

fn to_vec<B: Backend, const D: usize>(t: Tensor<B, D>) -> (Vec<f32>, Vec<usize>) {
    let shape = t.dims().to_vec();
    (t.into_data().to_vec().expect("f32 data"), shape)
}

/// Largest absolute difference, and the largest one relative to the reference's largest magnitude.
fn diff(got: &[f32], want: &[f32]) -> (f32, f32) {
    assert_eq!(got.len(), want.len(), "lengths differ");
    let scale = want.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1e-12);
    let worst = got.iter().zip(want).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    (worst, worst / scale)
}

/// Compares the tensors along the way, where the golden file has them (relative to each one's largest magnitude).
fn intermediates<B: Backend>(name: &str, st: &SafeTensors<'_>, trace: &pb_classifier_roblox::Trace<B>) -> String {
    let mut line = String::new();
    if !st.names().contains(&"logmel") {
        return line;
    }
    for (key, got) in [
        ("logmel", to_vec(trace.logmel.clone())),
        ("conv2", to_vec(trace.conv2.clone())),
        ("pool4", to_vec(trace.pools[0].clone())),
        ("final_ln", to_vec(trace.final_ln.clone())),
    ] {
        let (want, shape) = floats(st, key);
        let mut got_shape = got.1.clone();
        got_shape.remove(0);
        assert_eq!(got_shape, shape, "{name}: {key} shape");
        let (abs, rel) = diff(&got.0, &want);
        line += &format!("  {key} {abs:.1e}/{rel:.1e}");
        assert!(rel <= 1e-3, "{name}: {key} differs (abs {abs}, relative {rel})");
    }
    line
}

#[derive(serde::Deserialize)]
struct Index {
    cases: std::collections::BTreeMap<String, Case>,
}

#[derive(serde::Deserialize)]
struct Case {
    samples: usize,
    windows: usize,
}

#[test]
#[ignore = "needs the model weights (PB_WEIGHTS); run with --ignored"]
fn matches_the_pytorch_reference_on_cpu() {
    let started = std::time::Instant::now();
    let clf =
        RobloxClassifier::<Cpu>::load_cpu(&model_dir(), NonZeroUsize::new(8).expect("non-zero")).expect("model loads");
    eprintln!("loaded in {:?}: {:?}", started.elapsed(), clf);
    let index: Index =
        serde_json::from_str(&std::fs::read_to_string(golden("index.json")).expect("index")).expect("json");

    let mut worst_prob = 0.0f32;
    for (name, case) in &index.cases {
        let bytes = std::fs::read(golden(&format!("{name}.safetensors"))).expect("golden file");
        let st = SafeTensors::deserialize(&bytes).expect("safetensors");
        let pcm = audio(&st);
        assert_eq!(pcm.len(), case.samples);
        if case.windows > 0 {
            // Long audio: the caller splits it into 30 s windows (25 s hop); each window must match.
            for w in 0..case.windows {
                let start_len =
                    pb_testkit::golden::i64s(st.tensor(&format!("w{w}_start_len")).expect("window bounds").data());
                let (start, len) = (start_len[0] as usize, start_len[1] as usize);
                let trace = clf.trace(&pcm[start..start + len]).expect("scores the window");
                let (probs, _) = to_vec(sigmoid(trace.logits));
                let (want, _) = floats(&st, &format!("w{w}_probs"));
                let (d, _) = diff(&probs, &want);
                worst_prob = worst_prob.max(d);
                eprintln!("{name} window {w}: |Δp| ≤ {d:.2e}");
                assert!(d <= 1e-4, "{name} window {w}: label probabilities differ by {d}");
            }
            continue;
        }
        let t0 = std::time::Instant::now();
        let trace = clf.trace(&pcm).expect("scores the clip");
        let (probs, _) = to_vec(sigmoid(trace.logits.clone()));
        let (lang, _) = to_vec(softmax(trace.language_logits.clone(), 0));
        let took = t0.elapsed();
        let (want_p, _) = floats(&st, "probs");
        let (want_l, _) = floats(&st, "language_probs");
        let (dp, _) = diff(&probs, &want_p);
        let (dl, _) = diff(&lang, &want_l);
        worst_prob = worst_prob.max(dp);
        let argmax = |v: &[f32]| v.iter().enumerate().fold(0, |b, (i, x)| if *x > v[b] { i } else { b });
        let mut line = format!(
            "{name:<12} {:>5.2}s in {took:>9.2?}  |Δp| ≤ {dp:.2e}  |Δlang| ≤ {dl:.2e}",
            case.samples as f32 / 16000.0
        );
        line += &intermediates(name, &st, &trace);
        eprintln!("{line}");
        assert!(dp <= 1e-4, "{name}: label probabilities differ by {dp}");
        assert!(dl <= 1e-4, "{name}: language probabilities differ by {dl}");
        assert_eq!(argmax(&lang), argmax(&want_l), "{name}: top language");
    }
    eprintln!("worst label probability difference: {worst_prob:.2e}");
}

#[test]
#[ignore = "needs the model weights (PB_WEIGHTS); run with --ignored"]
fn honours_the_classifier_contract() {
    let mut clf =
        RobloxClassifier::<Cpu>::load_cpu(&model_dir(), NonZeroUsize::new(4).expect("non-zero")).expect("model loads");
    let bytes = std::fs::read(golden("profane_1.safetensors")).expect("golden file");
    let st = SafeTensors::deserialize(&bytes).expect("safetensors");
    pb_models_api::contract::classifier(&mut clf, &audio(&st));
    assert_eq!(
        clf.info().min_samples,
        480,
        "3 mel frames: the shortest input with a token left at every stage"
    );
    assert_eq!(clf.info().max_samples, 480_000);
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "needs the model weights (PB_WEIGHTS) and a GPU; run with --ignored --features gpu"]
fn matches_the_pytorch_reference_on_gpu() {
    use pb_classifier_roblox::{Gpu, GpuDevice};
    let started = std::time::Instant::now();
    let clf =
        RobloxClassifier::<Gpu>::load_gpu(&model_dir(), GpuDevice::DiscreteGpu(0)).expect("model loads on the GPU");
    eprintln!("loaded in {:?}: {:?}", started.elapsed(), clf);
    let index: Index =
        serde_json::from_str(&std::fs::read_to_string(golden("index.json")).expect("index")).expect("json");
    for (name, case) in &index.cases {
        if case.windows > 0 {
            continue;
        }
        let bytes = std::fs::read(golden(&format!("{name}.safetensors"))).expect("golden file");
        let st = SafeTensors::deserialize(&bytes).expect("safetensors");
        let pcm = audio(&st);
        let _ = clf.trace(&pcm).expect("warm-up");
        let t0 = std::time::Instant::now();
        let trace = clf.trace(&pcm).expect("scores the clip");
        let (probs, _) = to_vec(sigmoid(trace.logits.clone()));
        let (lang, _) = to_vec(softmax(trace.language_logits.clone(), 0));
        let took = t0.elapsed();
        let (dp, _) = diff(&probs, &floats(&st, "probs").0);
        let (dl, _) = diff(&lang, &floats(&st, "language_probs").0);
        eprintln!(
            "gpu {name:<12} {:>5.2}s in {took:>9.2?}  |Δp| ≤ {dp:.2e}  |Δlang| ≤ {dl:.2e}{}",
            case.samples as f32 / 16000.0,
            intermediates(name, &st, &trace)
        );
        assert!(
            dp <= 1e-3 && dl <= 1e-3,
            "{name}: GPU differs from the reference ({dp}, {dl})"
        );
    }
}
