//! The conveyor: which station a sentence's card shows. A pure function of the sentence's stage timestamps and the
//! server time, so every tab (and a tab that was hidden for an hour) draws the same picture without replaying.
//!
//! A card visits each station the sentence visited, stays at least [`DWELL_MS`] at each so fast stages stay visible,
//! is never ahead of the truth, and never passes a station the sentence did not visit.

use serde::{Deserialize, Serialize};

use super::wire::ServerMs;

/// The stations, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Station {
    Recording,
    Cut,
    Queued,
    Model,
    Verdict,
    Decision,
}

impl Station {
    pub const ALL: [Station; 6] = [
        Station::Recording,
        Station::Cut,
        Station::Queued,
        Station::Model,
        Station::Verdict,
        Station::Decision,
    ];

    pub fn index(self) -> usize {
        self as usize
    }
}

/// The least time a card shows each station before it may move on (Recording, Cut, Queued, Model, Verdict).
pub const DWELL_MS: [ServerMs; 5] = [0, 300, 300, 400, 700];
/// How long a dropped sentence stays on the belt.
pub const DROPPED_VISIBLE_MS: ServerMs = 4_000;

/// When a sentence reached each stage (server clock).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamps {
    pub opened: ServerMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut: Option<ServerMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped: Option<ServerMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued: Option<ServerMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scoring: Option<ServerMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scored: Option<ServerMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided: Option<ServerMs>,
    /// Scoring failed; the card goes straight to the decision station.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed: Option<ServerMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_start: Option<ServerMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_end: Option<ServerMs>,
}

impl Stamps {
    /// The stations this sentence has reached, with when (in station order).
    fn visits(&self) -> impl Iterator<Item = (Station, ServerMs)> {
        let after_cut = self.dropped.is_none();
        [
            (Station::Recording, Some(self.opened)),
            (Station::Cut, self.dropped.or(self.cut)),
            (Station::Queued, self.queued.filter(|_| after_cut)),
            (Station::Model, self.scoring.filter(|_| after_cut)),
            (
                Station::Verdict,
                self.scored.filter(|_| after_cut && self.failed.is_none()),
            ),
            (Station::Decision, self.failed.or(self.decided).filter(|_| after_cut)),
        ]
        .into_iter()
        .filter_map(|(s, t)| t.map(|t| (s, t)))
    }
}

/// What a card shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shown {
    pub station: Station,
    /// When the card arrived at `station` (for "waiting 300 ms" counters and animations).
    pub since: ServerMs,
    pub dropped: bool,
    pub failed: bool,
    /// The card has left the belt (a dropped sentence after [`DROPPED_VISIBLE_MS`]).
    pub gone: bool,
    /// The bot is playing its warning for this sentence right now.
    pub playing: bool,
}

/// The station a card shows at `now`.
pub fn display_stage(s: &Stamps, now: ServerMs) -> Shown {
    let mut shown = (Station::Recording, s.opened);
    let mut prev: Option<(Station, ServerMs)> = None;
    for (station, t) in s.visits() {
        let arrive = match prev {
            Some((p, a)) => t.max(a + DWELL_MS.get(p.index()).copied().unwrap_or(0)),
            _ => t,
        };
        if arrive > now {
            break;
        }
        shown = (station, arrive);
        prev = Some((station, arrive));
    }
    let dropped = s.dropped.is_some_and(|d| d <= now) && shown.0 == Station::Cut;
    Shown {
        station: shown.0,
        since: shown.1,
        dropped,
        failed: s.failed.is_some() && shown.0 == Station::Decision,
        gone: dropped && now >= shown.1 + DROPPED_VISIBLE_MS,
        playing: s.play_start.is_some_and(|p| p <= now) && s.play_end.is_none_or(|e| e > now),
    }
}
