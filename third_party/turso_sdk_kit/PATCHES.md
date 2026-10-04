# Local patches to turso_sdk_kit 0.8.1

turso_sdk_kit 0.8.1 from crates.io (MIT, Turso Inc.), used through `[patch.crates-io]` in the workspace `Cargo.toml`.
Only the manifest changed; the Rust sources are as published.

1. `turso_core`: `default-features = false` with every default feature except `simd` (the `simsimd` C library, see
   `third_party/turso/PATCHES.md`), plus the features this crate already asked for.
2. The `bindgen` build-dependency is removed: `build.rs` never uses it (it only compiles a Windows version resource;
   `bindgen.sh` is run by hand upstream), and it would pull `clang-sys` into every build.

Drop this copy once upstream makes `simd` opt-in and drops the unused build-dependency.
