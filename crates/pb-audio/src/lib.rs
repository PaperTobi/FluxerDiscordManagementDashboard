//! Audio handling: decoding any common format (pure Rust: symphonia, and opus-decoder for Opus in Ogg/WebM),
//! WAV, resampling, loudness (EBU R128), peak limiting, fades, and the preparation of voice clips.

pub mod v1;

pub use v1::*;
