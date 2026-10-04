//! Helpers for tests across the workspace. Never part of the shipped program.

pub mod audio;
pub mod golden;
#[cfg(feature = "livekit")]
pub mod lk;
#[cfg(feature = "memvoice")]
pub mod memvoice;
#[cfg(feature = "fakemodels")]
pub mod models;

use std::path::PathBuf;

/// A file in `crates/pb-testkit/fixtures`.
pub fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures").join(name)
}

/// The model weights and voices for the tests that need them: `PB_WEIGHTS`, else `target/weights` in the workspace
/// (where `cargo run -p pb -- fetch-weights --dest target/weights` puts them).
pub fn weights() -> PathBuf {
    std::env::var_os("PB_WEIGHTS").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/weights"),
        PathBuf::from,
    )
}
