//! Old history, violations and evidence → `sentence` records.

use std::collections::{BTreeMap, VecDeque};

use jiff::Timestamp;
use pb_domain::{BlobHash, ChannelId, ClfLang, GuildId, Label, SentenceId, UserId};
use pb_store_api::{CutCause, DecisionRecord, SentenceRecord, SentenceSource};

use super::old::{OldDb, Row};

/// The old classifier: the same Roblox model.
pub(crate) const OLD_MODEL: &str = "roblox-voice-safety-v3 (old bot)";

pub fn ts(secs: f64) -> Timestamp {
    Timestamp::from_millisecond((secs * 1000.0).round() as i64).unwrap_or(Timestamp::UNIX_EPOCH)
}

fn label(s: &str) -> Option<Label> {
    s.parse().ok()
}

/// The old bot's scores (`{"profanity": 0.93, …}`) by label; a label it does not know is left out. `None` when
/// there are none or they cannot be read.
pub(crate) fn scores(json: Option<&str>) -> Option<[f32; 8]> {
    let map: BTreeMap<String, f64> = serde_json::from_str(json?).ok()?;
    let mut out = [0.0; 8];
    for (k, v) in map {
        if let Some(l) = label(&k) {
            out[l.index()] = v as f32;
        }
    }
    Some(out)
}

fn language(code: Option<&str>) -> ClfLang {
    code.and_then(ClfLang::from_code).unwrap_or(ClfLang::Other)
}

/// `strike 1 of 2` → (1, 2); `hourly cap of 5 reached` → 5.
fn strike(reason: &str) -> Option<(u32, u32)> {
    let rest = reason.strip_prefix("strike ")?;
    let (a, b) = rest.split_once(" of ")?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn cap(reason: &str) -> Option<u32> {
    reason
        .strip_prefix("hourly cap of ")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Converts the history; `audio` maps old evidence ids to their blobs. `window` is the escalation window used to
/// recount violations (the old bot did not store the count). Returns the records and how many sentences without a
/// history row were made from evidence alone.
pub fn convert(
    old: &OldDb,
    audio: &BTreeMap<String, BlobHash>,
    window: Option<f64>,
) -> (Vec<(Timestamp, SentenceRecord)>, u64, Vec<String>) {
    let mut notes = Vec::new();
    let violations: BTreeMap<String, &Row> = old
        .violations
        .iter()
        .filter_map(|r| r.text(9).map(|c| (c, r)))
        .collect();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut out: Vec<(Timestamp, SentenceRecord)> = Vec::new();
    let mut counts: BTreeMap<(GuildId, UserId), VecDeque<f64>> = BTreeMap::new();
    let mut count = |g: GuildId, u: UserId, t: f64| -> u32 {
        let q = counts.entry((g, u)).or_default();
        q.push_back(t);
        if let Some(w) = window {
            while q.front().is_some_and(|f| t - f > w) {
                q.pop_front();
            }
        }
        q.len() as u32
    };
    // History rows, plus evidence-only sentences, in time order.
    let mut items: Vec<(f64, Option<&Row>, Option<&Row>)> = old
        .history
        .iter()
        .map(|h| (h.real(0).unwrap_or(0.0), Some(h), None))
        .collect();
    let history_uids: std::collections::BTreeSet<String> = old.history.iter().filter_map(|h| h.text(4)).collect();
    for v in &old.violations {
        if v.text(9).is_none_or(|c| !history_uids.contains(&c)) {
            items.push((v.real(0).unwrap_or(0.0), None, Some(v)));
        }
    }
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut orphans = 0;
    for (t, h, v_only) in items {
        let (guild, channel, user, uid) = match (h, v_only) {
            (Some(h), _) => (h.text(1), h.text(2), h.text(3), h.text(4)),
            (None, Some(v)) => (v.text(1), v.text(2), v.text(3), v.text(9)),
            _ => continue,
        };
        let (Some(guild), Some(channel), Some(user)) = (
            guild.and_then(|g| g.parse::<GuildId>().ok()),
            channel.and_then(|c| c.parse::<ChannelId>().ok()),
            user.and_then(|u| u.parse::<UserId>().ok()),
        ) else {
            notes.push(format!(
                "a history row at {} without a community, channel or person was skipped",
                ts(t)
            ));
            continue;
        };
        if let Some(u) = &uid
            && !seen.insert(u.clone())
        {
            continue;
        }
        let vio = h
            .and_then(|_| uid.as_ref().and_then(|u| violations.get(u).copied()))
            .or(v_only);
        let audio = vio.and_then(|v| v.text(10)).and_then(|e| audio.get(&e).copied());
        let record = match h {
            Some(h) => {
                let sc = scores(h.text(6).as_deref()).unwrap_or_default();
                let flagged: Vec<Label> = h
                    .text(7)
                    .and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|l| label(l))
                    .collect();
                let decision_text = h.text(8).unwrap_or_default();
                let reason = h.text(9).unwrap_or_default();
                let step = h.int(11).and_then(|s| u32::try_from(s).ok()).unwrap_or(1);
                let (vlabel, vscore) = vio
                    .map(|v| (v.text(4).and_then(|l| label(&l)), v.real(5)))
                    .and_then(|(l, s)| l.zip(s))
                    .unwrap_or_else(|| {
                        let l = flagged
                            .iter()
                            .copied()
                            .max_by(|a, b| sc[a.index()].total_cmp(&sc[b.index()]))
                            .unwrap_or(Label::Profanity);
                        (l, f64::from(sc[l.index()]))
                    });
                let decision = match decision_text.as_str() {
                    "play" => DecisionRecord::Warn {
                        label: vlabel,
                        score: vscore as f32,
                        step,
                        count: count(guild, user, t),
                    },
                    "observe" => DecisionRecord::Observe {
                        label: vlabel,
                        score: vscore as f32,
                        step,
                        count: count(guild, user, t),
                    },
                    _ if flagged.is_empty() => DecisionRecord::NothingFlagged,
                    _ if reason == "no longer tracked here" => DecisionRecord::NoLongerTracked,
                    _ => match (strike(&reason), cap(&reason)) {
                        (Some((s, of)), _) => DecisionRecord::Strike { strike: s, of },
                        (_, Some(per_hour)) => DecisionRecord::OldHourlyCap { per_hour },
                        _ => DecisionRecord::InvalidScore,
                    },
                };
                let thresholds = Vec::new();
                SentenceRecord {
                    id: SentenceId::new(),
                    guild,
                    channel,
                    user,
                    started: ts(t - h.real(5).unwrap_or(0.0)),
                    dur_ms: (h.real(5).unwrap_or(0.0) * 1000.0).round() as u32,
                    level_db: None,
                    cut: CutCause::Unknown,
                    scores: sc,
                    language: language(h.text(10).as_deref()),
                    thresholds,
                    flagged,
                    decision,
                    jar: false,
                    audio,
                    infer_ms: None,
                    cut_to_verdict_ms: None,
                    model: OLD_MODEL.into(),
                    source: SentenceSource::Import,
                }
            }
            None => {
                let Some(v) = v_only else { continue };
                orphans += 1;
                let Some(l) = v.text(4).and_then(|l| label(&l)) else {
                    continue;
                };
                let score = v.real(5).unwrap_or(0.0) as f32;
                let mut sc = [0.0f32; 8];
                sc[l.index()] = score;
                let dur = old
                    .evidence
                    .iter()
                    .find(|e| e.text(0) == v.text(10))
                    .and_then(|e| e.real(7))
                    .unwrap_or(0.0);
                SentenceRecord {
                    id: SentenceId::new(),
                    guild,
                    channel,
                    user,
                    started: ts(t - dur),
                    dur_ms: (dur * 1000.0).round() as u32,
                    level_db: None,
                    cut: CutCause::Unknown,
                    scores: sc,
                    language: ClfLang::Other,
                    thresholds: Vec::new(),
                    flagged: vec![l],
                    decision: DecisionRecord::Warn {
                        label: l,
                        score,
                        step: v.int(6).and_then(|s| u32::try_from(s).ok()).unwrap_or(1),
                        count: count(guild, user, t),
                    },
                    jar: false,
                    audio,
                    infer_ms: None,
                    cut_to_verdict_ms: None,
                    model: OLD_MODEL.into(),
                    source: SentenceSource::Import,
                }
            }
        };
        out.push((ts(t), record));
    }
    (out, orphans, notes)
}
