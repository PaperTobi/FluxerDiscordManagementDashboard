//! Per-frame speech probabilities → sentence spans. An exact port of the old bot's `segmenter.py` (the Roblox
//! project's segmenter): IDLE → PENDING (speech seen, not yet long enough) → SPEECH (a sentence is open) → cut → IDLE,
//! or straight into the next sentence after a cut at the maximum length. Everything is in absolute sample indices;
//! frame `k` covers samples `[k·512, (k+1)·512)`.

use serde::{Deserialize, Serialize};

use super::{FRAME, FRAME_MS, ms_to_frames, ms_to_samples};

const F: i64 = FRAME as i64;
const FMS: i64 = FRAME_MS as i64;

/// Segmenter parameters (the original `SegCfg`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegCfg {
    /// Speech probability that starts looking at a sentence.
    pub start_thr: f64,
    /// Below this a frame counts as silence (hysteresis).
    pub end_thr: f64,
    /// Voiced time needed before a sentence opens / to keep it.
    pub min_voiced_ms: u32,
    /// Silence while PENDING that aborts it (a "blip").
    pub abort_gap_ms: u32,
    /// Audio kept before the onset.
    pub pre_roll_ms: u32,
    /// Silence kept after the last voiced frame.
    pub tail_ms: u32,
    /// Pause that ends a sentence.
    pub end_silence_ms: u32,
    /// Longest sentence; longer speech is cut at a quiet spot.
    pub max_clip_s: f64,
    /// Where to look for that quiet spot.
    pub soft_window_ms: u32,
    /// Shortest quiet run that counts.
    pub soft_min_run_ms: u32,
}

/// The original segmenter's defaults (`roblox_vc`). The bot sets end silence, maximum length and minimum voiced time
/// from its settings (`pb_engine::track::segcfg`), whose defaults are the old bot's.
impl Default for SegCfg {
    fn default() -> Self {
        SegCfg {
            start_thr: 0.50,
            end_thr: 0.35,
            min_voiced_ms: 300,
            abort_gap_ms: 160,
            pre_roll_ms: 300,
            tail_ms: 250,
            end_silence_ms: 700,
            max_clip_s: 15.0,
            soft_window_ms: 2000,
            soft_min_run_ms: 96,
        }
    }
}

impl SegCfg {
    /// The two relations the original `clamp()` enforces between fields: the end threshold is at most the start
    /// threshold, and the soft window at most half the maximum length.
    pub fn normalized(mut self) -> Self {
        self.end_thr = self.end_thr.min(self.start_thr);
        self.soft_window_ms = (f64::from(self.soft_window_ms).min(self.max_clip_s * 1000.0 / 2.0)) as u32;
        self
    }
}

/// Why a sentence ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutReason {
    Silence,
    MaxSoft,
    MaxHard,
    Disconnect,
    Stall,
    Mute,
    Close,
    End,
}

/// Why the stream was flushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlushReason {
    Disconnect,
    Stall,
    Mute,
    /// The track ended (the bot left the call or stops).
    Close,
    /// The input ended (a recording played through, or the bot stops).
    End,
}

impl From<FlushReason> for CutReason {
    fn from(r: FlushReason) -> Self {
        match r {
            FlushReason::Disconnect => CutReason::Disconnect,
            FlushReason::Stall => CutReason::Stall,
            FlushReason::Mute => CutReason::Mute,
            FlushReason::Close => CutReason::Close,
            FlushReason::End => CutReason::End,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Open {
    pub id: u64,
    pub s0: i64,
    /// The sentence this one continues after a cut at the maximum length.
    pub continues: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    pub id: u64,
    pub s0: i64,
    pub s1: i64,
    /// Voiced core.
    pub v0: i64,
    pub v1: i64,
    pub reason: CutReason,
    pub voiced_ms: i64,
    /// Sample index when the decision was taken.
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub id: u64,
    pub s0: i64,
    pub s1: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Open(Open),
    Cut(Cut),
    /// Too little voice: the sentence is not scored.
    Drop(Dropped),
    /// Speech that never reached a sentence.
    Blip {
        s0: i64,
        s1: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegState {
    Idle,
    Pending,
    Speech,
}

/// The segmenter. `new_id` hands out sentence ids.
pub struct Segmenter<I: FnMut() -> u64> {
    cfg: SegCfg,
    new_id: I,
    state: SegState,
    prev_end: i64,
    hist: Vec<(i64, f64, bool)>,
    id: Option<u64>,
    s0: i64,
    v0: i64,
    last_voice: Option<i64>,
    sil: i64,
    cvoiced: i64,
    first: i64,
    pvoiced: i64,
    pgap: i64,
    last_pv: i64,
}

impl<I: FnMut() -> u64> std::fmt::Debug for Segmenter<I> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Segmenter")
            .field("state", &self.state)
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl<I: FnMut() -> u64> Segmenter<I> {
    pub fn new(cfg: SegCfg, new_id: I, base: i64) -> Self {
        let mut s = Segmenter {
            cfg,
            new_id,
            state: SegState::Idle,
            prev_end: base,
            hist: Vec::new(),
            id: None,
            s0: 0,
            v0: 0,
            last_voice: None,
            sil: 0,
            cvoiced: 0,
            first: 0,
            pvoiced: 0,
            pgap: 0,
            last_pv: 0,
        };
        s.reset(base);
        s
    }

    fn reset(&mut self, base: i64) {
        self.state = SegState::Idle;
        self.prev_end = base;
        self.hist.clear();
        self.id = None;
        self.s0 = 0;
        self.v0 = 0;
        self.last_voice = None;
        self.sil = 0;
        self.cvoiced = 0;
        self.first = 0;
        self.pvoiced = 0;
        self.pgap = 0;
        self.last_pv = 0;
    }

    /// Takes effect on the next frame.
    pub fn reconfigure(&mut self, cfg: SegCfg) {
        self.cfg = cfg;
    }

    pub fn state(&self) -> SegState {
        self.state
    }

    /// First sample the open sentence (or a pending one, with pre-roll) can still need.
    pub fn earliest_needed(&self) -> i64 {
        match self.state {
            SegState::Speech => self.s0,
            SegState::Pending => (self.first * F - ms_to_samples(f64::from(self.cfg.pre_roll_ms)))
                .max(self.prev_end)
                .max(0),
            SegState::Idle => self.prev_end,
        }
    }

    fn max_samples(&self) -> i64 {
        ms_to_samples(self.cfg.max_clip_s * 1000.0)
    }

    /// One frame's speech probability.
    pub fn push(&mut self, k: i64, p: f64) -> Vec<Event> {
        let mut out = Vec::new();
        let voiced = p >= self.cfg.end_thr;

        if self.state == SegState::Idle {
            if p < self.cfg.start_thr {
                return out;
            }
            self.state = SegState::Pending;
            self.first = k;
            self.pvoiced = 0;
            self.pgap = 0;
            self.last_pv = k;
            self.hist.clear();
        }

        if self.state == SegState::Pending {
            self.hist.push((k, p, voiced));
            if voiced {
                self.pvoiced += 1;
                self.pgap = 0;
                self.last_pv = k;
            } else {
                self.pgap += 1;
            }
            if self.pvoiced * FMS >= i64::from(self.cfg.min_voiced_ms) {
                out.push(self.open_from_pending(k));
                if (k + 1) * F - self.s0 >= self.max_samples() {
                    out.extend(self.cut_max(k));
                }
            } else if self.pgap * FMS >= i64::from(self.cfg.abort_gap_ms) {
                out.push(Event::Blip {
                    s0: self.first * F,
                    s1: (self.last_pv + 1) * F,
                });
                self.state = SegState::Idle;
                self.hist.clear();
            }
            return out;
        }

        self.hist.push((k, p, voiced));
        if voiced {
            if self.cvoiced == 0 {
                self.v0 = k * F;
            }
            self.last_voice = Some(k);
            self.sil = 0;
            self.cvoiced += 1;
        } else {
            self.sil += 1;
        }
        let now = (k + 1) * F;
        if self.sil * FMS >= i64::from(self.cfg.end_silence_ms) {
            out.extend(self.cut_silence(now));
        } else if now - self.s0 >= self.max_samples() {
            out.extend(self.cut_max(k));
        }
        out
    }

    /// The stream stopped: closes whatever is open. `now` is the sample index of the end of the stream.
    pub fn flush(&mut self, now: i64, reason: FlushReason) -> Vec<Event> {
        let mut out = Vec::new();
        match self.state {
            SegState::Speech => {
                let (s1, v1) = match self.last_voice {
                    Some(last) => {
                        let v1 = (last + 1) * F;
                        (now.min(v1 + ms_to_samples(f64::from(self.cfg.tail_ms))), v1)
                    }
                    None => (now, now),
                };
                out.extend(self.emit(s1, v1, self.cvoiced, reason.into(), now));
            }
            SegState::Pending => out.push(Event::Blip {
                s0: self.first * F,
                s1: (self.last_pv + 1) * F,
            }),
            SegState::Idle => {}
        }
        self.state = SegState::Idle;
        self.hist.clear();
        self.id = None;
        out
    }

    fn open_from_pending(&mut self, k: i64) -> Event {
        let id = (self.new_id)();
        self.id = Some(id);
        self.s0 = (self.first * F - ms_to_samples(f64::from(self.cfg.pre_roll_ms)))
            .max(self.prev_end)
            .max(0);
        self.v0 = self.first * F;
        self.cvoiced = self.pvoiced;
        self.last_voice = Some(self.last_pv);
        self.sil = k - self.last_pv;
        self.state = SegState::Speech;
        Event::Open(Open {
            id,
            s0: self.s0,
            continues: None,
        })
    }

    fn emit(&mut self, s1: i64, v1: i64, voiced_frames: i64, reason: CutReason, now: i64) -> Option<Event> {
        let id = self.id?;
        let voiced_ms = voiced_frames * FMS;
        let ev = if voiced_ms < i64::from(self.cfg.min_voiced_ms) || s1 <= self.s0 {
            Event::Drop(Dropped {
                id,
                s0: self.s0,
                s1: s1.max(self.s0),
            })
        } else {
            Event::Cut(Cut {
                id,
                s0: self.s0,
                s1,
                v0: self.v0,
                v1: v1.max(self.v0),
                reason,
                voiced_ms,
                now,
            })
        };
        self.prev_end = self.prev_end.max(s1);
        Some(ev)
    }

    fn cut_silence(&mut self, now: i64) -> Vec<Event> {
        let (s1, v1) = match self.last_voice {
            Some(last) => {
                let v1 = (last + 1) * F;
                ((v1 + ms_to_samples(f64::from(self.cfg.tail_ms))).min(now), v1)
            }
            None => (now, now),
        };
        let out = self
            .emit(s1, v1, self.cvoiced, CutReason::Silence, now)
            .into_iter()
            .collect();
        self.state = SegState::Idle;
        self.hist.clear();
        self.id = None;
        out
    }

    /// Length cap reached: cut at the longest quiet run of the last ~2 s, else at the lowest-probability frame, else
    /// hard at the current frame.
    fn cut_max(&mut self, k: i64) -> Vec<Event> {
        let cfg = &self.cfg;
        let now = (k + 1) * F;
        let win = ms_to_frames(f64::from(cfg.soft_window_ms)).min(ms_to_frames(cfg.max_clip_s * 500.0));
        let first_inside = self.s0.div_euclid(F) + i64::from(self.s0.rem_euclid(F) != 0);
        let lo = (k - win + 1).max(first_inside);
        let cand: Vec<(i64, f64)> = self
            .hist
            .iter()
            .filter(|(kk, _, _)| *kk >= lo)
            .map(|(kk, pp, _)| (*kk, *pp))
            .collect();
        let min_run = ms_to_frames(f64::from(cfg.soft_min_run_ms));

        let mut runs: Vec<(i64, i64)> = Vec::new();
        let (mut start, mut prev): (Option<i64>, i64) = (None, 0);
        for &(kk, pp) in &cand {
            if pp < cfg.end_thr {
                if start.is_none() {
                    start = Some(kk);
                }
                prev = kk;
            } else if let Some(s) = start.take() {
                runs.push((s, prev));
            }
        }
        if let Some(s) = start {
            runs.push((s, prev));
        }
        runs.retain(|(a, b)| b - a + 1 >= min_run);

        let (mut cut, mut reason) = (now, CutReason::MaxHard);
        if let Some(&(a, b)) = runs.iter().fold(None::<&(i64, i64)>, |best, r| match best {
            Some(b) if (b.1 - b.0, b.0) >= (r.1 - r.0, r.0) => Some(b),
            _ => Some(r),
        }) {
            cut = (a + (b - a + 1) / 2) * F;
            reason = CutReason::MaxSoft;
        } else if let Some(&(kk, pp)) = cand.iter().fold(None::<&(i64, f64)>, |best, c| match best {
            Some(b) if (b.1, -b.0) <= (c.1, -c.0) => Some(b),
            _ => Some(c),
        }) && pp < cfg.start_thr
        {
            cut = kk * F;
            reason = CutReason::MaxSoft;
        }
        if cut - self.s0 < ms_to_samples(f64::from(cfg.min_voiced_ms)) {
            cut = now;
            reason = CutReason::MaxHard;
        }
        self.cut_at(cut, reason, now)
    }

    fn cut_at(&mut self, cut: i64, reason: CutReason, now: i64) -> Vec<Event> {
        let (head, rest): (Vec<_>, Vec<_>) = self.hist.iter().copied().partition(|e| e.0 * F < cut);
        let head_voiced: Vec<i64> = head.iter().filter(|e| e.2).map(|e| e.0).collect();
        let v1 = head_voiced.last().map_or(cut, |last| ((last + 1) * F).min(cut));
        let old_id = self.id;
        let mut out: Vec<Event> = self
            .emit(cut, v1, head_voiced.len() as i64, reason, now)
            .into_iter()
            .collect();

        let id = (self.new_id)();
        self.id = Some(id);
        self.s0 = cut;
        let rest_voiced: Vec<i64> = rest.iter().filter(|e| e.2).map(|e| e.0).collect();
        self.cvoiced = rest_voiced.len() as i64;
        self.last_voice = rest_voiced.last().copied();
        self.v0 = rest_voiced.first().map_or(cut, |first| first * F);
        self.sil = rest.iter().rev().take_while(|e| !e.2).count() as i64;
        self.hist = rest;
        self.state = SegState::Speech;
        out.push(Event::Open(Open {
            id,
            s0: cut,
            continues: old_id,
        }));
        out
    }
}
