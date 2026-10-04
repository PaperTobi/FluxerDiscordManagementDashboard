# Local patches to turso 0.8.1

turso 0.8.1 from crates.io (MIT, Turso Inc.), used through `[patch.crates-io]` in the workspace `Cargo.toml`. Only the
manifest changed; the Rust sources are as published.

Why: the published crate pulls `turso_core` with its default features, which include `simd` = the `simsimd` C library
(used only by the vector distance SQL functions; without it turso_core uses its pure-Rust implementation, the same one
it uses on WebAssembly). It also always pulls the sync SDK (`turso_sync_sdk_kit`, `turso_sync_engine`), which the
`sync` feature is for. The bot uses neither vector functions nor sync, and its dependencies must be pure Rust
(docs/exceptions.toml).

1. `turso_core`: `default-features = false` with every default feature except `simd`
   (`fs uuid time json series percentile autovacuum encryption`).
2. `turso_sync_sdk_kit`: optional, enabled by the `sync` feature (its `pure-rust-crypto` forwarding became `?/`).

Drop this copy once upstream makes `simd` and the sync SDK opt-in (to be proposed upstream).
