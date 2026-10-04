//! The audio formats the parts of the bot agree on.

/// What the bot listens to: mono at 16 kHz (the rate of the voice-activity model and the classifier).
pub const LISTEN_RATE: u32 = 16_000;

/// What the bot plays: mono at 48 kHz.
pub const PLAY_RATE: u32 = 48_000;

/// Samples per analysis frame at [`LISTEN_RATE`] (one voice-activity step).
pub const FRAME: usize = 512;

/// One frame in milliseconds.
pub const FRAME_MS: u32 = 32;

const _: () = assert!(FRAME as u32 * 1000 == FRAME_MS * LISTEN_RATE);
