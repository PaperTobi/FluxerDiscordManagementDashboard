//! Waiting between tries and pacing what Fluxer limits (shared by the REST client and the gateway).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// The wait before try number `n` (from 0): `base` doubling up to `max`, give or take a quarter (so clients that failed
/// together do not come back together).
pub(crate) fn backoff(base: Duration, max: Duration, n: u32) -> Duration {
    base.saturating_mul(1 << n.min(16))
        .min(max)
        .mul_f64(0.75 + fastrand::f64() * 0.5)
}

/// At most `n` sends in any `window` (a rolling window).
#[derive(Debug)]
pub(crate) struct Pace {
    sent: VecDeque<Instant>,
    n: usize,
    window: Duration,
}

impl Pace {
    pub(crate) fn new(n: usize, window: Duration) -> Pace {
        Pace {
            sent: VecDeque::new(),
            n: n.max(1),
            window,
        }
    }

    /// When the next send may go (`None` = now).
    pub(crate) fn next(&mut self) -> Option<Instant> {
        let now = Instant::now();
        while self.sent.front().is_some_and(|t| now.duration_since(*t) >= self.window) {
            self.sent.pop_front();
        }
        if self.sent.len() < self.n {
            None
        } else {
            self.sent.front().map(|t| *t + self.window)
        }
    }

    pub(crate) fn record(&mut self) {
        self.sent.push_back(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_up_to_the_most() {
        let ms = |n| backoff(Duration::from_millis(100), Duration::from_secs(1), n).as_millis();
        assert!((75..=125).contains(&ms(0)));
        assert!((150..=250).contains(&ms(1)));
        assert!((750..=1250).contains(&ms(10)));
        assert!((750..=1250).contains(&ms(u32::MAX)));
    }

    #[test]
    fn pace_lets_n_through_per_window() {
        let mut p = Pace::new(2, Duration::from_secs(60));
        for _ in 0..2 {
            assert!(p.next().is_none());
            p.record();
        }
        assert!(p.next().is_some());
    }
}
