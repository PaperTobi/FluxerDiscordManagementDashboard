/// Remembers when the bot last played something into the call, so its own voice (picked up by someone's microphone)
/// is never scored as theirs (`PlaybackGuard`). Times are monotonic seconds.
#[derive(Debug, Clone, PartialEq)]
pub struct EchoGuard {
    /// How long after playback ends speech still counts as echo.
    pub pad_s: f64,
    start: Option<f64>,
    end: Option<f64>,
}

impl Default for EchoGuard {
    fn default() -> Self {
        EchoGuard::new(1.0)
    }
}

impl EchoGuard {
    pub fn new(pad_s: f64) -> Self {
        EchoGuard {
            pad_s,
            start: None,
            end: None,
        }
    }

    pub fn begin(&mut self, now: f64) {
        self.start = Some(now);
        self.end = None;
    }

    pub fn end(&mut self, now: f64) {
        if self.start.is_some() {
            self.end = Some(now);
        }
    }

    /// When the current (or last) playback started.
    pub fn started(&self) -> Option<f64> {
        self.start
    }

    /// Whether `[t0, t1]` overlaps the playback (until `pad_s` after it ended; open-ended while playing).
    pub fn overlaps(&self, t0: f64, t1: f64) -> bool {
        let Some(start) = self.start else { return false };
        let end = self.end.map_or(f64::INFINITY, |e| e + self.pad_s);
        t1 >= start && t0 <= end
    }
}

/// `_trim_playback`: of a sentence spanning `[t0, t1]` that overlaps playback starting at `playback_start`, the
/// number of leading samples to keep (the speech before the bot started talking), or `None` when less than
/// `min_keep_s` would remain. Speech after the playback is never kept: its echo can still be in it.
pub fn keep_before_playback(t0: f64, playback_start: Option<f64>, sample_rate: u32, min_keep_s: f64) -> Option<usize> {
    let start = playback_start?;
    if start <= t0 {
        return None;
    }
    let keep = ((start - t0) * f64::from(sample_rate)) as usize;
    (keep as f64 >= f64::from(sample_rate) * min_keep_s).then_some(keep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_while_playing_and_for_the_pad() {
        let mut g = EchoGuard::new(1.0);
        assert!(!g.overlaps(0.0, 10.0));
        g.begin(5.0);
        assert!(g.overlaps(4.0, 5.0));
        assert!(g.overlaps(100.0, 101.0), "open-ended while playing");
        g.end(7.0);
        assert!(g.overlaps(7.9, 9.0));
        assert!(!g.overlaps(8.1, 9.0));
        assert!(!g.overlaps(1.0, 4.9));
    }

    #[test]
    fn keeps_speech_before_playback_only() {
        assert_eq!(keep_before_playback(1.0, Some(3.0), 16_000, 0.6), Some(32_000));
        assert_eq!(keep_before_playback(1.0, Some(1.5), 16_000, 0.6), None);
        assert_eq!(keep_before_playback(2.0, Some(1.0), 16_000, 0.6), None);
    }
}
