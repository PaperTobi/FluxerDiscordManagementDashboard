//! The moderation actor: every scored sentence is decided here, in order (strikes, observe-only, late verdicts,
//! escalation), recorded, and turned into a warning, reports and moderation actions.

use std::sync::Arc;

use bytes::Bytes;
use jiff::Timestamp;
use pb_domain::PlayPurpose;
use pb_domain::{Audience, GuildId, Label, SentenceId, UserId};
use pb_infer::Scored;
use pb_live_proto::{CutWhy, DecisionView, PersonDelta, SentenceCard, Stamps, VerdictView, ViolationItem, Who};
use pb_policy::{Chan, ClearReason, DecideInput, Decider, Decision, Violations};
use pb_settings::Recordings;
use pb_store_api::{BlobAdded, BlobRole, CutCause, DecisionRecord, Event, SentenceRecord, SentenceSource};
use pb_voicelines::{Field, Fields, Line, Sel};

use super::core::{Core, PlayItem, RoomHandle};
use super::mailbox::Mailbox;
use super::supervise::{ActorError, Life, Policy, Supervised};

/// A sentence after the classifier.
#[derive(Debug)]
pub struct Heard {
    pub id: SentenceId,
    pub no: u32,
    pub chan: Chan,
    pub user: UserId,
    pub room: RoomHandle,
    pub started: Timestamp,
    pub dur_ms: u32,
    pub level_db: f32,
    pub cut: CutCause,
    pub cut_why: CutWhy,
    pub pcm: Arc<[f32]>,
    pub scored: Result<Scored, pb_infer::InferError>,
    /// When the sentence was cut (monotonic seconds).
    pub cut_mono: f64,
    pub stamps: Stamps,
}

#[derive(Debug)]
pub enum ModMsg {
    Heard(Box<Heard>),
    /// The swear jar was emptied.
    JarReset(GuildId, UserId),
    /// A person's counts now (for their page).
    Counts(GuildId, UserId, tokio::sync::oneshot::Sender<pb_live_proto::Counts>),
}

/// A person's counts: the swear jar, violations in the escalation window and since midnight (reporting time zone),
/// and the step the next violation is on.
fn counts(core: &Core, violations: &Violations, g: GuildId, u: UserId) -> pb_live_proto::Counts {
    let eff = core.settings.current().effective(Some(g), Some(u));
    let now = core.deps.clock.now();
    let now_mono = core.deps.clock.mono();
    let window = eff.violation_window.value.value().map(|d| d.get().secs());
    let in_window = violations.count(g, u, now_mono, window);
    let tz = eff.timezone.value.zone();
    let since_midnight = now
        .to_zoned(tz)
        .start_of_day()
        .map(|m| now.duration_since(m.timestamp()).as_secs_f64())
        .unwrap_or(0.0);
    pb_live_proto::Counts {
        jar: core.jar(g, u),
        in_window,
        window_ms: eff.violation_window.value.value().map(|d| d.get().millis()),
        today: violations.count(g, u, now_mono, Some(since_midnight)),
        step: eff.escalation.value.step_for(in_window + 1).map_or(0, |(n, _)| n),
    }
}

fn ms(t: Timestamp) -> i64 {
    t.as_millisecond()
}

/// The decision as the conveyor shows it.
pub fn decision_view(d: &DecisionRecord) -> DecisionView {
    match *d {
        DecisionRecord::NothingFlagged | DecisionRecord::OldHourlyCap { .. } => DecisionView::NothingFlagged,
        DecisionRecord::InvalidScore => DecisionView::InvalidScore,
        DecisionRecord::NoLongerTracked => DecisionView::NoLongerTracked,
        DecisionRecord::Strike { strike, of } => DecisionView::Strike { strike, of },
        DecisionRecord::Warn { step, count, .. } => DecisionView::Warn { step, count },
        DecisionRecord::Observe { step, count, .. } => DecisionView::Observe { step, count },
        DecisionRecord::Late { step, count, .. } => DecisionView::Late { step, count },
    }
}

/// The model's name for the record.
fn model_name(core: &Core) -> String {
    core.deps.inference.classifier_info().model.clone()
}

/// Decides every scored sentence, in order.
pub(crate) struct Moderation {
    decider: Decider,
    violations: Violations,
}

impl Supervised for Moderation {
    type Ctx = Arc<Core>;
    type Msg = ModMsg;
    const NAME: &'static str = "moderation";
    const POLICY: Policy = Policy::Restart;

    /// Every violation so far, for the escalation counts: any community or person may count over any window, and
    /// windows can be lengthened later (one number per violation). Strikes start over, as after a restart of the bot.
    async fn start(core: &Arc<Core>) -> Result<Self, ActorError> {
        core.index_caught_up().await;
        let mut violations = Violations::default();
        let (mono, now) = (core.deps.clock.mono(), core.deps.clock.now());
        for (g, u, t) in core.deps.index.violation_times().await? {
            let ago = now.duration_since(t).as_secs_f64().max(0.0);
            violations.seed(g, u, mono - ago);
        }
        Ok(Moderation {
            decider: Decider::default(),
            violations,
        })
    }

    async fn run(mut self, core: Arc<Core>, mb: &mut Mailbox<ModMsg>, life: Life) -> Result<(), ActorError> {
        loop {
            let msg = tokio::select! {
                m = mb.recv() => match m {
                    Some(m) => m,
                    None => return Ok(()),
                },
                () = life.cancel.cancelled() => return Ok(()),
            };
            match msg {
                ModMsg::JarReset(g, u) => {
                    if let Ok(mut j) = core.jar.lock() {
                        j.insert((g, u), 0);
                    }
                }
                ModMsg::Heard(h) => decide(&core, &mut self.decider, &mut self.violations, *h).await,
                ModMsg::Counts(g, u, reply) => {
                    let _ = reply.send(counts(&core, &self.violations, g, u));
                }
            }
        }
    }
}

async fn decide(core: &Arc<Core>, decider: &mut Decider, violations: &mut Violations, h: Heard) {
    let (g, u) = (h.chan.guild, h.user);
    let tree = core.settings.current();
    let eff = tree.effective(Some(g), Some(u));
    let now = core.deps.clock.now();
    let now_mono = core.deps.clock.mono();
    let mut stamps = h.stamps;
    stamps.scored = Some(ms(now));
    let base_card = SentenceCard {
        id: h.id,
        no: h.no,
        stamps,
        dur_ms: Some(h.dur_ms),
        level_db: Some(h.level_db),
        cut: Some(h.cut_why),
        dropped: None,
        error: None,
        verdict: None,
        decision: None,
    };
    let scored = match h.scored {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(guild = %g, user = %u, error = %e, "a sentence could not be scored");
            let mut card = base_card;
            card.stamps.failed = Some(ms(now));
            card.error = Some(e.to_string());
            core.live.sentence(g, u, &card);
            return;
        }
    };
    let raw = &scored.raw;
    let enabled = eff.enabled_labels();
    let thresholds: Vec<(Label, f32)> = enabled.iter().map(|l| (*l, eff.threshold_for(*l) as f32)).collect();
    let mut flagged: Vec<Label> = thresholds
        .iter()
        .filter(|(l, t)| raw.label(*l) >= *t)
        .map(|(l, _)| *l)
        .collect();
    flagged.sort_by(|a, b| raw.label(*b).total_cmp(&raw.label(*a)));
    let finite = raw.labels.iter().all(|x| x.is_finite());
    let still_tracked = tree.is_tracked(g, u);
    let late = eff
        .max_reaction_delay
        .value
        .value()
        .is_some_and(|d| now_mono - h.cut_mono > d.get().secs());
    let input = DecideInput {
        flagged: !flagged.is_empty(),
        finite,
        still_tracked,
        observe_only: eff.observe_only.value,
        strikes: eff.strikes.value.get(),
        strike_window: eff.strike_window.value.value().map(|d| d.get().secs()),
        late,
    };
    let decision = decider.decide(g, u, &input, now_mono);
    let language = raw.top_language();
    let top = flagged.first().copied();
    let window = eff.violation_window.value.value().map(|d| d.get().secs());
    let (record, step_info) = match (&decision, top) {
        (Decision::Warn | Decision::Observe | Decision::Late, Some(label)) => {
            let count = violations.record(g, u, now_mono, window);
            let (step_no, step) = eff
                .escalation
                .value
                .step_for(count)
                .map_or((1, None), |(n, s)| (n, Some(s.clone())));
            let score = raw.label(label);
            let rec = match decision {
                Decision::Warn => DecisionRecord::Warn {
                    label,
                    score,
                    step: step_no,
                    count,
                },
                Decision::Observe => DecisionRecord::Observe {
                    label,
                    score,
                    step: step_no,
                    count,
                },
                _ => DecisionRecord::Late {
                    label,
                    score,
                    step: step_no,
                    count,
                },
            };
            (rec, step.map(|s| (step_no, count, s)))
        }
        (Decision::Strike { strike, of }, _) => (
            DecisionRecord::Strike {
                strike: *strike,
                of: *of,
            },
            None,
        ),
        (
            Decision::Clear {
                reason: ClearReason::InvalidScore,
            },
            _,
        ) => (DecisionRecord::InvalidScore, None),
        (
            Decision::Clear {
                reason: ClearReason::NoLongerTracked,
            },
            _,
        ) => (DecisionRecord::NoLongerTracked, None),
        _ => (DecisionRecord::NothingFlagged, None),
    };
    let violation = record.is_violation();
    let jar = violation && eff.jar_enabled.value;
    // Keep the recording of flagged sentences (or of every sentence).
    let keep_audio = match eff.recordings.value {
        Recordings::Off => false,
        Recordings::Flagged => !flagged.is_empty(),
        Recordings::All => true,
    };
    let wav = Bytes::from(pb_audio::wav16(&pb_audio::to_i16(&h.pcm), pb_voice_api::LISTEN_RATE));
    let mut events = Vec::new();
    let mut audio = None;
    if keep_audio {
        match core.deps.blobs.put(wav.clone()).await {
            Ok(info) => {
                if info.new {
                    events.push(Event::BlobAdded(BlobAdded {
                        hash: info.hash,
                        size: info.size,
                        media_type: "audio/wav".into(),
                        role: BlobRole::Recording,
                    }));
                }
                audio = Some(info.hash);
            }
            Err(e) => tracing::error!(error = %e, "a recording could not be kept"),
        }
    }
    let sentence = SentenceRecord {
        id: h.id,
        guild: g,
        channel: h.chan.channel,
        user: u,
        started: h.started,
        dur_ms: h.dur_ms,
        level_db: Some(h.level_db),
        cut: h.cut,
        scores: raw.labels,
        language,
        thresholds: thresholds.clone(),
        flagged: flagged.clone(),
        decision: record,
        jar,
        audio,
        infer_ms: Some(scored.infer_ms),
        cut_to_verdict_ms: Some(u32::try_from(((now_mono - h.cut_mono) * 1000.0).max(0.0) as u64).unwrap_or(u32::MAX)),
        model: model_name(core),
        source: SentenceSource::Live,
    };
    events.push(Event::Sentence(Box::new(sentence.clone())));
    core.record(events).await;
    if jar && let Ok(mut j) = core.jar.lock() {
        *j.entry((g, u)).or_insert(0) += 1;
    }

    // The conveyor.
    let mut card = base_card;
    card.stamps.decided = Some(ms(core.deps.clock.now()));
    card.verdict = Some(VerdictView {
        scores: raw.labels,
        thresholds,
        flagged: flagged.clone(),
        language,
        infer_ms: scored.infer_ms,
        cut_to_verdict_ms: sentence.cut_to_verdict_ms.unwrap_or(0),
    });
    card.decision = Some(decision_view(&record));
    core.live.sentence(g, u, &card);
    let counts = counts(core, violations, g, u);
    core.live.person(g, u, PersonDelta::Counts { counts });
    if let Some((label, score, step, count)) = record.violation() {
        let (who, community, channel) = {
            let gs = core.guilds();
            let p = gs.person(g, u);
            (
                Who {
                    user: u,
                    name: gs.name(g, u),
                    avatar: core.avatar_url(u, p.and_then(|p| p.avatar.as_deref())),
                },
                gs.guild_name(g),
                pb_live_proto::ChannelRef {
                    id: h.chan.channel,
                    name: gs.channel_name(g, h.chan.channel),
                },
            )
        };
        core.live.violation(ViolationItem {
            sentence: h.id,
            guild: g,
            community,
            channel,
            who,
            label,
            score,
            step,
            count,
            decision: decision_view(&record),
            at_ms: ms(now),
        });
    }

    // What the bot says.
    let deadline = eff
        .max_reaction_delay
        .value
        .value()
        .map(|d| h.cut_mono + d.get().secs());
    let mut fields = Fields::new();
    if let Some((step, count, _)) = &step_info {
        fields.insert(Field::Step, step.to_string());
        fields.insert(Field::Count, count.to_string());
    }
    fields.insert(Field::Strikes, eff.strikes.value.get().to_string());
    match (&decision, top) {
        (Decision::Warn, Some(label)) => {
            let step = step_info.as_ref().map_or(1, |s| s.0);
            h.room.play(PlayItem {
                line: Line::Warning {
                    label: Sel::Is(label),
                    step: Sel::Is(step),
                },
                person: Some(u),
                audience: Audience::from(eff.audience.value),
                fields: fields.clone(),
                purpose: PlayPurpose::Warning,
                sentence: Some(h.id),
                deadline,
                by: None,
                text: None,
                heard: Some(language),
                label: Some(label),
                done: None,
            });
        }
        (Decision::Strike { strike, of }, _) if eff.strike_notice.value => {
            let mut f = fields.clone();
            f.insert(Field::Count, strike.to_string());
            f.insert(Field::Strikes, of.to_string());
            h.room.play(PlayItem {
                line: Line::StrikeNotice,
                person: Some(u),
                audience: Audience::from(eff.audience.value),
                fields: f,
                purpose: PlayPurpose::StrikeNotice,
                sentence: Some(h.id),
                deadline,
                by: None,
                text: None,
                heard: Some(language),
                label: top,
                done: None,
            });
        }
        _ => {}
    }

    // The action, then one report with its result (in their own task: Fluxer may be slow).
    if !flagged.is_empty() && still_tracked {
        let core2 = core.clone();
        let s2 = sentence.clone();
        let room = h.room.clone();
        let step = step_info.map(|(_, _, step)| step);
        tokio::spawn(async move {
            let action = match step.as_ref().and_then(|st| st.action.kind().map(|k| (st, k))) {
                Some((st, kind)) => {
                    Some(super::actions::step_action(&core2, &s2, kind, st, room, Some(language)).await)
                }
                None => None,
            };
            super::reports::flagged(&core2, &s2, step.as_ref(), action.as_ref(), &wav).await;
        });
    }
}
