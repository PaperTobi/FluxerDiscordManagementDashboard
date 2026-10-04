//! What to do with a scored sentence: strikes, warnings, observe-only, late verdicts, and violation counting for the
//! escalation steps. No hourly cap and no cooldowns.

use std::collections::{BTreeMap, VecDeque};

use pb_domain::{GuildId, UserId};
use serde::Serialize;

/// The decision for one sentence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    /// Nothing flagged, or the person is no longer tracked here.
    Clear { reason: ClearReason },
    /// Flagged, but the strike count is not reached yet.
    Strike { strike: u32, of: u32 },
    /// A violation: warn now.
    Warn,
    /// A violation in observe-only mode: recorded, nothing played.
    Observe,
    /// A violation whose verdict came too late to react to: recorded and counted, nothing played.
    Late,
}

/// Why a sentence needs nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClearReason {
    NothingFlagged,
    /// The model returned a score that is not a number.
    InvalidScore,
    NoLongerTracked,
}

impl Decision {
    /// Whether this sentence counts as a violation (escalation, swear jar, reports).
    pub fn is_violation(&self) -> bool {
        matches!(self, Decision::Warn | Decision::Observe | Decision::Late)
    }
}

/// The settings and facts a decision needs.
#[derive(Debug, Clone, PartialEq)]
pub struct DecideInput {
    pub flagged: bool,
    /// All scores were finite.
    pub finite: bool,
    pub still_tracked: bool,
    pub observe_only: bool,
    pub strikes: u32,
    /// Seconds; `None` = unlimited.
    pub strike_window: Option<f64>,
    /// The verdict arrived later than the allowed reaction delay.
    pub late: bool,
}

/// Strike counting per person and community.
#[derive(Debug, Clone, Default)]
pub struct Decider {
    strikes: BTreeMap<(GuildId, UserId), VecDeque<f64>>,
}

impl Decider {
    pub fn decide(&mut self, guild: GuildId, user: UserId, input: &DecideInput, now: f64) -> Decision {
        // A score that is not a number never reaches a bar, so this comes first.
        if !input.finite {
            return Decision::Clear {
                reason: ClearReason::InvalidScore,
            };
        }
        if !input.flagged {
            return Decision::Clear {
                reason: ClearReason::NothingFlagged,
            };
        }
        if !input.still_tracked {
            return Decision::Clear {
                reason: ClearReason::NoLongerTracked,
            };
        }
        let hist = self.strikes.entry((guild, user)).or_default();
        hist.push_back(now);
        if let Some(window) = input.strike_window {
            while hist.front().is_some_and(|t| now - t > window) {
                hist.pop_front();
            }
        }
        let n = hist.len() as u32;
        if n < input.strikes.max(1) {
            return Decision::Strike {
                strike: n,
                of: input.strikes,
            };
        }
        hist.clear();
        if input.late {
            Decision::Late
        } else if input.observe_only {
            Decision::Observe
        } else {
            Decision::Warn
        }
    }

    /// Forgets a person's strikes (e.g. after a settings reset).
    pub fn forget(&mut self, guild: GuildId, user: UserId) {
        self.strikes.remove(&(guild, user));
    }
}

/// Violation times per person and community (seeded from the history at start).
#[derive(Debug, Clone, Default)]
pub struct Violations {
    times: BTreeMap<(GuildId, UserId), Vec<f64>>,
}

impl Violations {
    pub fn seed(&mut self, guild: GuildId, user: UserId, t: f64) {
        self.times.entry((guild, user)).or_default().push(t);
    }

    /// Records a violation and returns how many fall in the window (including this one).
    pub fn record(&mut self, guild: GuildId, user: UserId, now: f64, window: Option<f64>) -> u32 {
        self.times.entry((guild, user)).or_default().push(now);
        self.count(guild, user, now, window)
    }

    /// Violations within `window` seconds before `now` (`None` = all).
    pub fn count(&self, guild: GuildId, user: UserId, now: f64, window: Option<f64>) -> u32 {
        self.times.get(&(guild, user)).map_or(0, |v| {
            v.iter().filter(|t| window.is_none_or(|w| now - **t <= w)).count() as u32
        })
    }

    pub fn reset(&mut self, guild: GuildId, user: UserId) {
        self.times.remove(&(guild, user));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(strikes: u32) -> DecideInput {
        DecideInput {
            flagged: true,
            finite: true,
            still_tracked: true,
            observe_only: false,
            strikes,
            strike_window: Some(20.0),
            late: false,
        }
    }

    #[test]
    fn strikes_then_a_warning_then_strikes_again() {
        let mut d = Decider::default();
        let (g, u) = (GuildId(1), UserId(2));
        assert_eq!(d.decide(g, u, &input(2), 0.0), Decision::Strike { strike: 1, of: 2 });
        assert_eq!(d.decide(g, u, &input(2), 5.0), Decision::Warn);
        assert_eq!(d.decide(g, u, &input(2), 6.0), Decision::Strike { strike: 1, of: 2 });
        assert_eq!(
            d.decide(g, u, &input(2), 30.0),
            Decision::Strike { strike: 1, of: 2 },
            "the first strike expired"
        );
    }

    #[test]
    fn observe_only_late_and_untracked() {
        let mut d = Decider::default();
        let (g, u) = (GuildId(1), UserId(2));
        assert_eq!(
            d.decide(
                g,
                u,
                &DecideInput {
                    observe_only: true,
                    ..input(1)
                },
                0.0
            ),
            Decision::Observe
        );
        assert_eq!(
            d.decide(g, u, &DecideInput { late: true, ..input(1) }, 1.0),
            Decision::Late
        );
        assert!(matches!(
            d.decide(
                g,
                u,
                &DecideInput {
                    still_tracked: false,
                    ..input(1)
                },
                2.0
            ),
            Decision::Clear { .. }
        ));
        assert!(matches!(
            d.decide(
                g,
                u,
                &DecideInput {
                    flagged: false,
                    ..input(1)
                },
                3.0
            ),
            Decision::Clear { .. }
        ));
    }

    #[test]
    fn counts_violations_in_the_window() {
        let mut v = Violations::default();
        let (g, u) = (GuildId(1), UserId(2));
        v.seed(g, u, 0.0);
        assert_eq!(v.record(g, u, 100.0, Some(3600.0)), 2);
        assert_eq!(v.record(g, u, 5000.0, Some(3600.0)), 1);
        assert_eq!(v.count(g, u, 5000.0, None), 3);
    }
}
