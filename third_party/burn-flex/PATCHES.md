# Local patches to burn-flex 0.21.0

This is burn-flex 0.21.0 from crates.io (Apache-2.0 OR MIT, the Burn project) with the changes below, used through
`[patch.crates-io]` in the workspace `Cargo.toml`. Each change keeps the results identical to the original
sequential code (same per-element and per-head computation); only the work is spread over the rayon thread pool.
Drop this copy once upstream Burn ships equivalent changes (to be proposed upstream).

Why: measured 2026-10-03 on a Ryzen 5 7600X, 4 threads, for the Roblox voice-safety classifier (550 tokens):
element-wise ops (GELU's erf) ran single-threaded and took 20 ms per layer (PyTorch 2.7 ms), and fused attention ran
the 16 heads one after another, 37 ms per layer (PyTorch 22.5 ms).

1. `src/ops/unary.rs`: `unary_op_typed` maps large contiguous buffers in parallel chunks of 16 Ki elements
   (`map_in_place`); the closures gain `Send + Sync` bounds.
2. `src/ops/attention.rs`: `attention_impl` (flash) and `attention_naive_impl` run the independent (batch, head) pairs
   in parallel, each with its own scratch buffers, instead of one after another with shared buffers.
