//! Version 1.

mod echo;
mod framing;
mod ring;
mod segmenter;
mod windows;

pub use echo::{EchoGuard, keep_before_playback};
pub use framing::FrameAssembler;
pub use ring::PcmRing;
pub use segmenter::{Cut, CutReason, Dropped, Event, FlushReason, Open, SegCfg, SegState, Segmenter};
pub use windows::{Window, windows};

pub use pb_domain::{FRAME, FRAME_MS, LISTEN_RATE};

/// Python's `round()` (half to even), which the original code used for every ms → samples/frames conversion.
pub fn round_half_even(x: f64) -> i64 {
    x.round_ties_even() as i64
}

/// `ms_to_samples`: `round(ms * 16000 / 1000)`.
pub fn ms_to_samples(ms: f64) -> i64 {
    round_half_even(ms * f64::from(LISTEN_RATE) / 1000.0)
}

/// `ms_to_frames`: `max(1, round(ms / 32))`.
pub fn ms_to_frames(ms: f64) -> i64 {
    round_half_even(ms / f64::from(FRAME_MS)).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_like_python() {
        assert_eq!(round_half_even(2.5), 2);
        assert_eq!(round_half_even(3.5), 4);
        assert_eq!(round_half_even(-2.5), -2);
        assert_eq!(round_half_even(62.5), 62);
        assert_eq!(ms_to_frames(2000.0), 62);
        assert_eq!(ms_to_frames(10.0), 1);
        assert_eq!(ms_to_samples(600.0), 9600);
    }
}
