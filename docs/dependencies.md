# Dependencies: current and maintained

Every third-party crate must be on its newest release and actively maintained. A crate whose repository is
archived or gone, or that has had neither a release nor a commit for a year, is replaced, or its entry here says why
it stays. Two checks enforce this:

- `cargo xtask freshness` checks every crate our manifests name directly. It looks at crates.io (newest release, its
  age, whether our version requirement allows it) and the crate's GitHub repository (archived or missing, last push;
  set `GITHUB_TOKEN` for the full rate limit). It fails on anything not explained in `xtask/freshness.toml`. Run it
  before adding or bumping a crate.
- `cargo deny check` runs in `cargo xtask ci` and covers the whole tree: RustSec advisories (vulnerable,
  unmaintained, yanked), licences, sources and duplicate versions. Its configuration is `deny.toml`.
- `cargo shear` runs in `cargo xtask ci` too: a crate a manifest names but no code uses (or uses only in tests) fails
  it. A crate named only to turn on its features says so under `[package.metadata.cargo-shear]`.

A new dependency is judged before it is added: newest version, maintenance (recent releases or commits, issues being
answered, more than one person or an organisation behind it), adoption, and no C (see `docs/exceptions.toml`).

## Audit of 2026-10-04

All 65 direct crates were checked against crates.io and GitHub, and the full tree against the RustSec advisory
database.

### Changed

| Crate | Was | Now | Why |
|---|---|---|---|
| reqwest | 0.12.28 | 0.13.5, `rustls-no-provider` | Newest major. 0.12's rustls feature hard-wired `ring` (C and assembly) into our own HTTPS, contrary to the exceptions register. 0.13 lets us bring the crypto provider (see pb-tls). |
| tokio-tungstenite | 0.29 | 0.30 | Newest major; the gateway now connects with pb-tls's configuration. |
| tower-http | 0.6 | 0.7.1 | Newest major. |
| (new) pb-tls | — | rustls 0.23 + rustls-graviola 0.4 + rustls-platform-verifier 0.7 + webpki-root-certs 1 | One TLS client setup for the Fluxer API, the gateway and downloads. Graviola (by rustls' author) is Rust plus formally verified assembly, no C. Certificates are checked against the system's roots (a private CA for a self-hosted instance works) plus Mozilla's (a bare container works). A CPU without the needed instructions is reported at start. |
| console_error_panic_hook | 0.1.7 | removed | Repository archived (the rustwasm organisation was retired). Replaced by a three-line panic hook in pb-web. |
| hound | 3.5.1 | removed | No release since 2023. WAV is written by `pb_audio::wav16` and laid out by `pb_audio::wav_layout` (both small and tested); everything else is decoded by symphonia, which was already used. |
| parking_lot | 0.12.5 | removed from pb-voice-livekit | The standard library's `Mutex` does the job. |
| burn, burn-flex, onnx-ir, protobuf | 0.21 / 3.7 in pb-vad-silero | removed from pb-vad-silero | The VAD is now a hand-written forward pass (Burn spent most of each 0.2 ms step dispatching tiny operations; the loops take about 0.02 ms). Leaves the tree: onnx-ir, protobuf (with -support, -parse, -codegen) and strum. Burn stays for the classifier. |
| (new in pb-vad-silero) rten-onnx, rten-simd | — | 0.27.0 (2026-10-02) | Both were already in the tree through rten (pb-tts-piper), so nothing new is compiled; see below. |
| branches (through turso_core) | 0.4.6 from crates.io | 0.4.6 patched in `third_party/branches` | On nightly its build script picks code that calls `core::intrinsics::abort`, which nightly renamed: the bot did not build on nightly. 0.5.1 follows the rename but turso_core 0.8.1 asks for 0.4; the patch takes the stable code on every channel (third_party/branches/PATCHES.md). Drop it once turso_core moves to 0.5. |
| (new) burn-cubecl, cubecl | — | 0.21.0, 0.10.0, in pb-classifier-roblox's `gpu` feature only | The classifier's own GPU kernels (`src/gpu.rs`: linear layers, layer normalisation, self-attention), which Burn's public tensor API cannot express. Same organisation (tracel-ai) and the same releases burn-wgpu already brings in (2026-09-22; burn 0.22 and cubecl 0.11 are still pre-releases), so the tree gains no crate. Confined to the classifier in `xtask/layers.toml`. |

### Kept, with low release activity

| Crate | Last release | Why it stays |
|---|---|---|
| fluent-bundle, fluent-syntax, unic-langid | 2025-05 | Mozilla's Fluent, used in Firefox. The repository is maintained (commits in 2026-09), and the format is stable. unic-langid is part of Fluent's API. |
| ebur128 | 2024-10 | Implements a fixed standard (EBU R128) and is complete. Its maintainer (sdroege, GStreamer) still commits (2025-12). |
| secrecy | 2024-10 | Part of iqlusion's crates monorepo, which is active (2026-09). |
| unicode-normalization | 2025-10 | The unicode-rs organisation is active (2026-09). |
| opus-decoder | 2026-03 | Pure Rust, `forbid(unsafe_code)`, passes all 12 RFC 8251 conformance vectors, repository active (2026-07). The alternative, ropus, has less use and `unsafe` hot loops. symphonia has no Opus decoder. |
| espeak-ng (Rust port) | 2026-09 | A test-only dependency: it records how the Rust port differs from C espeak-ng (the reason the C library is an exception). |

### Unmaintained crates deeper in the tree (`deny.toml`)

These come only through maintained upstream projects; each `deny.toml` entry names the release that removes it.

| Advisory | Crate | Comes from | Way out |
|---|---|---|---|
| RUSTSEC-2025-0141 | bincode 2.0.1 | burn-core 0.21 | burn 0.22 drops it (pre-release 0.22.0-pre.4 already does) |
| RUSTSEC-2026-0173 | proc-macro-error2 | rstml 0.12 ← leptos_macro 0.8 (build time) | rstml 0.13, used by leptos 0.9 |
| RUSTSEC-2024-0436 | paste (finished, archived) | leptos 0.8/0.9, gemm/pulp/macerator ← burn-flex | none released yet |
| RUSTSEC-2020-0163 | term_size | tokei 15 (xtask only, not shipped) | none released yet; tokei itself is maintained |

`ring` is still compiled. LiveKit's signalling (livekit-net) uses it, and so it switches on rustls' `ring` feature
for the whole build. Our own connections use graviola. See the `ring` entry in `docs/exceptions.toml`.

### Decided before adding (later milestones)

| Need | Chosen | Not chosen, because |
|---|---|---|
| Process configuration (`config.toml` + environment) | config (rust-cli/config-rs 0.15, organisation-maintained, 2026-09) | tier: its repository no longer exists (404) and it has about 270 downloads in 90 days. figment: no release since 2024-05. |
| Errors with source spans (settings files) | annotate-snippets (rust-lang, used by cargo; 2026-05) | miette: last release 2025-04 |
| Browser tests over CDP | chromiumoxide 0.9 (2026-02, active) | — |
| Daily log files | tracing-appender 0.2.5 (tokio-rs, 2026-04) | — |
| HTTPS for the web UI | tokio-rustls 0.26.6 (the rustls organisation, 2026-09), crypto through pb-tls (graviola); the TLS listener is ours (about 80 lines, handshakes concurrent) | axum-server: one more layer for what axum's `Listener` trait already allows |
| Terminal output | anstream 1.0, comfy-table 8, indicatif 0.18 (all released in 2026) | — |
| The engine's scenario tests (no weights, no LiveKit) | nothing new: pb-testkit's stand-in models (`fakemodels`) and in-process voice, the fake Fluxer, and crates already in the tree (tempfile 3.27, 2026-03, repository active 2026-10; futures; async-trait; tracing-subscriber; tokio's `test-util`) | — |
| Reading `silero_vad.onnx` | rten-onnx 0.27 (robertknight/rten, released 2026-10-02, repository active; no dependencies, `forbid(unsafe_code)`) | onnx-ir + protobuf: pulled burn-tensor and protobuf's code generator into the VAD for one file |
| Vector instructions for the VAD's loops, chosen at run time (AVX-512, AVX2 with FMA, Arm Neon, else a portable fallback) without `unsafe` in our code | rten-simd 0.27 (same project and release) | pulp 0.22 (also in the tree, through gemm): depends on the archived `paste`. `std::arch` directly: needs `unsafe`, which the workspace denies. Plain loops: compiled for the x86-64 baseline only (SSE2), about half the speed |
