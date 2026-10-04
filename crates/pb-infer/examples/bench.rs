//! Model timings (used for the comparison with the Python bot): `cargo run --release -p pb-infer --example bench --
//! <cpu|vad|tts> [threads]` with PB_WEIGHTS set (`--features gpu` adds `gpu`); prints one JSON line per measurement and
//! the process's peak memory.

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::time::Instant;

use pb_models_api::{Classifier, FRAME, SpeakOpts, TtsEngine, VadModel};

fn weights() -> PathBuf {
    PathBuf::from(std::env::var("PB_WEIGHTS").unwrap_or_else(|_| "target/weights".into()))
}

fn speech(seconds: f64) -> Vec<f32> {
    let jfk = pb_audio::decode_file(&pb_testkit::fixture("jfk.wav"), Some("wav")).expect("jfk.wav");
    let n = (seconds * 16_000.0) as usize;
    jfk.samples.iter().copied().cycle().take(n).collect()
}

fn peak_rss_mb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn stats(mut ms: Vec<f64>) -> (f64, f64) {
    ms.sort_by(f64::total_cmp);
    (ms[ms.len() / 2], ms[(ms.len() * 9 / 10).min(ms.len() - 1)])
}

fn classifier(mut c: Box<dyn Classifier>, runtime: &str, load_ms: f64) {
    println!(r#"{{"part":"classifier","runtime":"{runtime}","load_ms":{load_ms:.0}}}"#);
    for seconds in [1.0, 3.0, 8.0, 15.0, 30.0] {
        let pcm = speech(seconds);
        for _ in 0..2 {
            c.classify(&pcm).expect("classify");
        }
        let runs: Vec<f64> = (0..10)
            .map(|_| {
                let t = Instant::now();
                c.classify(&pcm).expect("classify");
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        let (median, p90) = stats(runs);
        println!(
            r#"{{"part":"classifier","runtime":"{runtime}","seconds":{seconds},"median_ms":{median:.1},"p90_ms":{p90:.1}}}"#
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let part = args.get(1).map_or("cpu", String::as_str);
    let threads = args.get(2).and_then(|t| t.parse().ok()).unwrap_or(4usize);
    let w = weights();
    match part {
        "cpu" => {
            let t = Instant::now();
            let c = pb_classifier_roblox::RobloxClassifier::load_cpu(
                &w.join("roblox-voice-safety-v3"),
                NonZeroUsize::new(threads).expect("threads"),
            )
            .expect("load");
            classifier(
                Box::new(c),
                &format!("rust-burn-cpu-{threads}t"),
                t.elapsed().as_secs_f64() * 1000.0,
            );
        }
        #[cfg(feature = "gpu")]
        "gpu" => {
            let t = Instant::now();
            let c = pb_classifier_roblox::RobloxClassifier::load_gpu(
                &w.join("roblox-voice-safety-v3"),
                pb_classifier_roblox::GpuDevice::DiscreteGpu(0),
            )
            .expect("load");
            classifier(Box::new(c), "rust-burn-vulkan", t.elapsed().as_secs_f64() * 1000.0);
        }
        "vad" => {
            let mut vad = pb_vad_silero::SileroVad::load(&w.join("silero-vad")).expect("vad");
            let pcm = speech(60.0);
            let (frames, _) = pcm.as_chunks::<FRAME>();
            let mut state = vad.new_state();
            let t = Instant::now();
            for f in frames {
                vad.step(std::slice::from_ref(f), &mut [&mut state]);
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            println!(
                r#"{{"part":"vad","runtime":"rust-burn-cpu","frames":{},"us_per_frame":{:.1},"streams_realtime":{:.0}}}"#,
                frames.len(),
                ms * 1000.0 / frames.len() as f64,
                60_000.0 / ms
            );
        }
        "tts" => {
            let mut tts =
                pb_tts_piper::PiperEngine::new(Path::new(pb_espeak::BUILD_DATA_DIR), &[w.join("voices")], threads)
                    .expect("piper");
            let opts = SpeakOpts {
                rate: 1.0,
                speaker: None,
                sentence_gap_ms: 0,
            };
            for (voice, text) in [
                ("en_US-lessac-medium", "Hey Max, watch your language, please."),
                (
                    "de_DE-thorsten-medium",
                    "Hallo Max, bitte achte auf deine Sprache. Das war jetzt schon die zweite Verwarnung heute.",
                ),
                ("en_US-lessac-high", "Hey Max, watch your language, please."),
            ] {
                let first = Instant::now();
                tts.synthesize(voice, text, &opts).expect("speak");
                let first_ms = first.elapsed().as_secs_f64() * 1000.0;
                let mut audio_s = 0.0;
                let runs: Vec<f64> = (0..5)
                    .map(|_| {
                        let t = Instant::now();
                        let s = tts.synthesize(voice, text, &opts).expect("speak");
                        audio_s = s.samples.len() as f64 / f64::from(s.sample_rate);
                        t.elapsed().as_secs_f64() * 1000.0
                    })
                    .collect();
                let (median, _) = stats(runs);
                println!(
                    r#"{{"part":"tts","runtime":"rust-rten-{threads}t","voice":"{voice}","first_ms":{first_ms:.0},"median_ms":{median:.1},"audio_s":{audio_s:.2},"rtf":{:.3}}}"#,
                    median / 1000.0 / audio_s
                );
            }
        }
        other => panic!("unknown part {other}"),
    }
    println!(r#"{{"peak_rss_mb":{:.0}}}"#, peak_rss_mb());
}
