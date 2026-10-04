//! Where the time goes: `cargo run --release --example profile -- <model dir> <threads> [seconds]`.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use pb_classifier_roblox::{Cpu, load_runner};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args
        .get(1)
        .map_or_else(|| pb_testkit::weights().join("roblox-voice-safety-v3"), PathBuf::from);
    let threads: usize = args.get(2).and_then(|t| t.parse().ok()).unwrap_or(4);
    let seconds: f32 = args.get(3).and_then(|t| t.parse().ok()).unwrap_or(11.0);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("pool");
    let device = Default::default();
    let (runner, _) = load_runner::<Cpu>(&dir, &device).expect("loads");
    let n = (seconds * 16_000.0) as usize;
    let pcm: Vec<f32> = (0..n)
        .map(|i| ((i as f32 * 0.01).sin() * 0.1) + ((i * 7919 % 1000) as f32 / 1000.0 - 0.5) * 0.05)
        .collect();
    let _ = NonZeroUsize::new(threads);
    pool.install(|| {
        let _ = runner.forward(&pcm);
        let (_, stages) = runner.forward_profiled(&pcm);
        let total: f64 = stages.iter().map(|(_, d)| d.as_secs_f64()).sum();
        let layers: f64 = stages
            .iter()
            .filter(|(s, _)| s.starts_with("layer"))
            .map(|(_, d)| d.as_secs_f64())
            .sum();
        for (name, d) in &stages {
            if !name.starts_with("layer") || name == "layer 0" || name == "layer 4" || name == "layer 5" {
                println!("{name:<14} {:>8.1} ms", d.as_secs_f64() * 1e3);
            }
        }
        println!("24 layers      {:>8.1} ms", layers * 1e3);
        println!(
            "total          {:>8.1} ms for {seconds} s audio, {threads} threads",
            total * 1e3
        );
    });
}
