//! Times each operation of one encoder layer: `cargo run --release --example layer_ops -- <model dir> <threads> <tokens>`.

use std::path::PathBuf;
use std::time::Instant;

use burn::tensor::activation::gelu;
use burn::tensor::backend::Backend;
use burn::tensor::module::attention;
use burn::tensor::ops::AttentionModuleOptions;
use burn::tensor::{Distribution, Tensor};
use pb_classifier_roblox::{Cpu, load_runner};

fn time<T>(name: &str, reps: u32, mut f: impl FnMut() -> T) -> T {
    let mut out = f();
    let _ = Cpu::sync(&Default::default());
    let t = Instant::now();
    for _ in 0..reps {
        out = f();
    }
    let _ = Cpu::sync(&Default::default());
    println!(
        "{name:<28} {:>8.2} ms",
        t.elapsed().as_secs_f64() * 1e3 / f64::from(reps)
    );
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&args[1]);
    let threads: usize = args[2].parse().expect("threads");
    let l: usize = args[3].parse().expect("tokens");
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .expect("pool");
    let device = Default::default();
    let (runner, _) = load_runner::<Cpu>(&dir, &device).expect("loads");
    let layer = &runner.model.layers[0];
    pool.install(|| {
        let x = Tensor::<Cpu, 3>::random([1, l, 1024], Distribution::Normal(0.0, 1.0), &device);
        let n = time("norm1 (LayerNorm)", 5, || layer.norm1.forward(x.clone()));
        let qkv = time("in_proj (1024→3072)", 5, || {
            layer.self_attn.in_proj.forward(n.clone())
        });
        let split = |i: usize| {
            qkv.clone()
                .narrow(2, i * 1024, 1024)
                .reshape([1, l, 16, 64])
                .swap_dims(1, 2)
        };
        let (q, k, v) = (split(0), split(1), split(2));
        let a = time("attention (16 heads)", 5, || {
            attention(
                q.clone(),
                k.clone(),
                v.clone(),
                None,
                None,
                AttentionModuleOptions::default(),
            )
        });
        let merged = a.swap_dims(1, 2).reshape([1, l, 1024]);
        time("out_proj (1024→1024)", 5, || {
            layer.self_attn.out_proj.forward(merged.clone())
        });
        let h = time("linear1 (1024→4096)", 5, || layer.linear1.forward(n.clone()));
        let g = time("gelu", 5, || gelu(h.clone()));
        time("linear2 (4096→1024)", 5, || layer.linear2.forward(g.clone()));
        let w = layer.linear1.weight.val();
        let flat = n.clone().reshape([l, 1024]);
        time("plain matmul [L,1024]x[1024,4096]", 5, || {
            flat.clone().matmul(w.clone())
        });
        let rx = Tensor::<Cpu, 2>::random([l, 1024], Distribution::Normal(0.0, 1.0), &device);
        let rw = Tensor::<Cpu, 2>::random([1024, 4096], Distribution::Normal(0.0, 1.0), &device);
        time("random contiguous matmul", 5, || rx.clone().matmul(rw.clone()));
        let rwt = Tensor::<Cpu, 2>::random([4096, 1024], Distribution::Normal(0.0, 1.0), &device).transpose();
        time("random matmul, rhs transposed view", 5, || {
            rx.clone().matmul(rwt.clone())
        });
        println!("weight shape {:?}", w.dims());
    });
}
