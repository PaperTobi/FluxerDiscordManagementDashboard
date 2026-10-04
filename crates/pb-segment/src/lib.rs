//! Turning a microphone stream into sentences: 32 ms frames, a PCM ring addressed by absolute sample index, the
//! hysteresis segmenter (an exact port of the old bot's `segmenter.py`), the echo guard that keeps the bot's own voice
//! from being scored, and how audio longer than the classifier's window is split. Pure: no I/O, no clock, no async.

pub mod v1;

pub use v1::*;
