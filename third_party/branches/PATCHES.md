# Local patch to branches 0.4.6

branches 0.4.6 from crates.io (MIT, Khashayar Fereidani), used through `[patch.crates-io]` in the workspace
`Cargo.toml`; turso_core 0.8.1 asks for `branches = "0.4.3"`.

Why: on a nightly compiler its build script switches to code that calls `core::intrinsics::abort`, which nightly
renamed (2026-10), so turso (and with it the bot) did not build on nightly. 0.5.1 follows the rename but turso_core
cannot use 0.5.

1. `build.rs`: always the stable code path (`core::hint::cold_path`, stable since Rust 1.95), whatever the channel.
2. `Cargo.toml`: the example, tests and benchmark (not copied) are no longer listed.

Drop this copy once turso_core depends on branches 0.5.1 or newer.
