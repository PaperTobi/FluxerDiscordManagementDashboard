# 0001 — How the voice-safety classifier runs

Date: 2026-10-03. Status: decided (internal detail; the user's rule: fix in depth, else an alternative that follows
the ideology, else the closest one).

## The user's words
"youre not changing anything about how the model works i hope … if it may genuently degrade performance why would you
change the model"; "if issues arise check if fixable indepth if not check if alternatives exist that follow ideology
if not check if alternatives exists that are close enough candle etc."

## What was decided
The Roblox voice-safety classifier v3 is written in Burn 0.21 (`crates/pb-classifier-roblox`) and loads the official
`model.safetensors` unchanged (no conversion, no second weights file). Same architecture and sizes as Roblox's
`inference.py`; the log-mel front end computes the DFT as a matrix product (Burn's STFT only takes power-of-two sizes;
the model's window is 400), which is the same transform. Golden tests against the PyTorch reference: label
probabilities within 3.4e-7 (limit 1e-4), intermediates within ~1e-6 relative.

## Speed, and what was fixed
Measured on a Ryzen 5 7600X (busy machine, load 7–12), same threads for both:

| | first port | after fixes | PyTorch + MKL |
|---|---|---|---|
| 11 s clip, 4 threads | 4.64 s | 0.93 s | 0.95 s |
| 30 s clip, 8 threads | 10.1 s | 2.5 s | 3.1 s |

1. Our configuration: burn's default features were off, which also switched off burn-flex's `rayon` and `simd`
   (single-threaded, scalar). Fixed by naming them (`crates/pb-classifier-roblox/Cargo.toml`).
2. burn-flex 0.21.0 itself ran element-wise maths single-threaded and attention heads one after another. Fixed in a
   vendored copy (`third_party/burn-flex`, `PATCHES.md`, registered in `docs/exceptions.toml`), results unchanged.
3. `x86-v4`: gemm's AVX-512 kernels, picked at run time by CPU detection (AVX2 otherwise).

## Alternatives looked at
- candle 0.11 (pure Rust, same `gemm` crate): matrix product 11–13 ms vs PyTorch 15 ms, but its GELU is also
  single-threaded (14.6 ms) and it has no AMD GPU backend. Not needed once Burn was fixed.
- burn-onnx: needs an ONNX export from PyTorch (Roblox ships none), i.e. Python/torch back in the pipeline.
- onnxruntime (C++): not pure Rust; not needed.

## How to undo
Remove the `[patch.crates-io]` entry to go back to stock burn-flex (slower, same numbers); the classifier sits behind
`pb_models_api::Classifier`, so another implementation can replace it and must pass the same contract and golden tests.
