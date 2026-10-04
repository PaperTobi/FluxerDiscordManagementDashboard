//! The page's estimate of the bot's clock.

use super::wire::ServerMs;

/// Server time = local time + offset. Each `Ping` gives a sample; the one with the smallest round trip among the
/// recent ones wins (its one-way delay is the most certain).
#[derive(Debug, Clone, Default)]
pub struct ClockOffset {
    samples: Vec<(u32, i64)>,
}

const SAMPLES: usize = 8;

impl ClockOffset {
    /// `server_ms` from a message received at `local_ms`; `rtt_ms` is the round trip, when known.
    pub fn observe(&mut self, server_ms: ServerMs, local_ms: i64, rtt_ms: Option<u32>) {
        let rtt = rtt_ms.unwrap_or(u32::MAX);
        let one_way = if rtt == u32::MAX { 0 } else { i64::from(rtt / 2) };
        self.samples.push((rtt, server_ms + one_way - local_ms));
        if self.samples.len() > SAMPLES {
            self.samples.remove(0);
        }
    }

    /// The offset to add to the local clock (0 before the first sample).
    pub fn offset(&self) -> i64 {
        self.samples.iter().min_by_key(|(rtt, _)| *rtt).map_or(0, |(_, o)| *o)
    }

    pub fn server_now(&self, local_ms: i64) -> ServerMs {
        local_ms + self.offset()
    }
}
