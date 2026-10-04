//! Moderation actions from the escalation steps (mute, disconnect, time out) and the timed undo of a mute, which
//! survives restarts (pending undos come back from the index).

use std::sync::Arc;
use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use pb_domain::PlayPurpose;
use pb_domain::{ActionKind, ActionOutcome, ClfLang, SentenceId};
use pb_fluxer_api::{ErrorKind, FluxerError, MemberPatch};
use pb_live_proto::{Activity, PersonDelta};
use pb_settings::EscalationStep;
use pb_store_api::{ActionRecord, Event, SentenceRecord};
use pb_voicelines::{Field, Fields, Line, Sel};

use super::core::{Core, PlayItem, RoomHandle};
use super::mailbox::Mailbox;
use super::supervise::{ActorError, Life, Policy, Supervised};

/// Waits before trying a failed undo again (the last repeats).
const UNDO_RETRY_S: &[u64] = &[60, 300, 900, 3600];

/// The permission an action needs (its stable name).
fn permission(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Mute | ActionKind::Unmute => "mute-members",
        ActionKind::Disconnect => "move-members",
        ActionKind::Timeout => "moderate-members",
    }
}

fn outcome(kind: ActionKind, r: Result<(), FluxerError>) -> ActionOutcome {
    match r {
        Ok(()) => ActionOutcome::Done,
        Err(e) if e.kind == ErrorKind::Forbidden || e.is_code("MISSING_PERMISSIONS") => ActionOutcome::NotAllowed {
            permission: permission(kind).into(),
        },
        Err(e) => ActionOutcome::Failed { error: e.to_string() },
    }
}

/// Runs a step's action for a violation and records it. Announces it when the settings say so.
pub async fn step_action(
    core: &Arc<Core>,
    s: &SentenceRecord,
    kind: ActionKind,
    step: &EscalationStep,
    room: RoomHandle,
    heard: Option<ClfLang>,
) -> ActionRecord {
    let (g, u) = (s.guild, s.user);
    let eff = core.settings.current().effective(Some(g), Some(u));
    let now = core.deps.clock.now();
    let dur = step.duration.get().get();
    let secs = Some(dur.as_secs());
    let mut record = ActionRecord {
        id: SentenceId::new(),
        guild: g,
        user: u,
        kind,
        secs: if kind == ActionKind::Disconnect { None } else { secs },
        sentence: Some(s.id),
        step: s.decision.violation().map(|v| v.2),
        outcome: ActionOutcome::Done,
        undo_at: None,
        undoes: None,
        retry_at: None,
    };
    let until = now
        .checked_add(SignedDuration::try_from(dur).unwrap_or(SignedDuration::ZERO))
        .unwrap_or(now);
    if !eff.actions_enabled.value {
        record.outcome = ActionOutcome::SkippedOff;
    } else if eff.observe_only.value {
        record.outcome = ActionOutcome::SkippedObserve;
    } else if let Some(ctl) = core.ctl() {
        // Someone already server-muted them: leave it (and never lift a mute the bot did not set).
        let already_muted = kind == ActionKind::Mute && core.voice().of_user(u).any(|v| v.guild == g && v.mute);
        // What Fluxer's audit log says, in the community's chat language.
        let reason = Some(pb_i18n::text(
            super::reports::locale(core, Some(g)),
            "action-audit-reason",
            &[("step", record.step.unwrap_or(1).into())],
        ));
        let patch = match kind {
            ActionKind::Mute => MemberPatch {
                mute: Some(true),
                reason,
                ..MemberPatch::default()
            },
            ActionKind::Disconnect => MemberPatch {
                disconnect: true,
                reason,
                ..MemberPatch::default()
            },
            _ => MemberPatch {
                timeout_until: Some(Some(until)),
                reason,
                ..MemberPatch::default()
            },
        };
        if already_muted {
            record.outcome = ActionOutcome::AlreadyMuted;
        } else {
            record.outcome = outcome(kind, ctl.patch_member(g, u, patch).await);
            if kind == ActionKind::Mute && record.outcome == ActionOutcome::Done {
                record.undo_at = Some(until);
            }
        }
    } else {
        record.outcome = ActionOutcome::NotConnected;
    }
    core.record(vec![Event::Action(Box::new(record.clone()))]);
    if record.undo_at.is_some() {
        let _ = core.undo.send(record.clone());
    }
    show(core, &record);
    announce(core, &room, &record, heard);
    record
}

/// Shows an action on the person's live page.
fn show(core: &Core, record: &ActionRecord) {
    core.live.person(
        record.guild,
        record.user,
        PersonDelta::Activity {
            item: Activity::Action {
                id: record.id.0.as_u64_pair().1,
                kind: record.kind,
                at_ms: core.deps.clock.now().as_millisecond(),
                secs: record.secs,
                result: record.outcome.clone(),
            },
        },
    );
}

/// Says in the room what was done, when the settings ask for it.
fn announce(core: &Core, room: &RoomHandle, record: &ActionRecord, heard: Option<ClfLang>) {
    let eff = core.settings.current().effective(Some(record.guild), Some(record.user));
    if record.outcome != ActionOutcome::Done || !eff.announce_actions.value {
        return;
    }
    let mut fields = Fields::new();
    if let Some(s) = record.secs {
        fields.insert(Field::Duration, s.to_string());
    }
    room.play(PlayItem {
        line: Line::Action {
            kind: Sel::Is(record.kind),
        },
        person: Some(record.user),
        audience: pb_domain::Audience::from(eff.audience.value),
        fields,
        purpose: PlayPurpose::ActionNotice,
        sentence: record.sentence,
        deadline: None,
        by: None,
        text: None,
        heard,
        label: None,
        done: None,
    });
}

/// Lifts timed mutes when they are due: the pending ones from the index, then each new one (from its mailbox). A
/// failed lift is tried again later, for as long as it takes (a mute must not stay because Fluxer was down).
pub(crate) struct Undo {
    /// When, what, and how many tries failed.
    due: Vec<(Timestamp, ActionRecord, usize)>,
}

impl Supervised for Undo {
    type Ctx = Arc<Core>;
    type Msg = ActionRecord;
    const NAME: &'static str = "undo";
    const POLICY: Policy = Policy::Restart;

    async fn start(core: &Arc<Core>) -> Result<Self, ActorError> {
        core.index_caught_up().await;
        let due = core
            .deps
            .index
            .pending_undos()
            .await?
            .into_iter()
            .filter_map(|a| a.undo_at.map(|t| (t, a, 0)))
            .collect();
        Ok(Undo { due })
    }

    async fn run(mut self, core: Arc<Core>, mb: &mut Mailbox<ActionRecord>, life: Life) -> Result<(), ActorError> {
        loop {
            let next = self.due.iter().map(|(t, _, _)| *t).min();
            let wait = next.map_or(Duration::from_secs(3600), |t| {
                let d = t.duration_since(core.deps.clock.now());
                Duration::try_from(d).unwrap_or(Duration::ZERO)
            });
            tokio::select! {
                r = mb.recv() => match r {
                    // After a restart the index already lists what was still in the mailbox.
                    Some(a) => {
                        if let Some(t) = a.undo_at
                            && !self.due.iter().any(|(_, d, _)| d.id == a.id)
                        {
                            self.due.push((t, a, 0));
                        }
                    }
                    None => return Ok(()),
                },
                () = tokio::time::sleep(wait) => {
                    life.beat();
                    let now = core.deps.clock.now();
                    let (ready, rest): (Vec<_>, Vec<_>) = self.due.drain(..).partition(|(t, _, _)| *t <= now);
                    self.due = rest;
                    for (_, a, tries) in ready {
                        let wait = UNDO_RETRY_S[tries.min(UNDO_RETRY_S.len() - 1)];
                        let retry_at = now.checked_add(SignedDuration::from_secs(i64::try_from(wait).unwrap_or(3600))).unwrap_or(now);
                        if !undo(&core, &a, retry_at).await {
                            self.due.push((retry_at, a, tries + 1));
                        }
                    }
                }
                () = life.cancel.cancelled() => return Ok(()),
            }
        }
    }
}

/// Lifts one mute; `false` = try again at `retry_at`.
async fn undo(core: &Arc<Core>, a: &ActionRecord, retry_at: Timestamp) -> bool {
    let Some(ctl) = core.ctl() else { return false };
    let r = ctl
        .patch_member(
            a.guild,
            a.user,
            MemberPatch {
                mute: Some(false),
                ..MemberPatch::default()
            },
        )
        .await;
    // A person who left the community or voice has nothing to lift.
    let gone = matches!(&r, Err(e) if e.kind == ErrorKind::NotFound || e.is_code("USER_NOT_IN_VOICE"));
    let o = if gone {
        ActionOutcome::Done
    } else {
        outcome(ActionKind::Unmute, r)
    };
    let retry = o != ActionOutcome::Done;
    let lifted = !gone && !retry;
    let rec = ActionRecord {
        id: SentenceId::new(),
        guild: a.guild,
        user: a.user,
        kind: ActionKind::Unmute,
        secs: None,
        sentence: a.sentence,
        step: a.step,
        outcome: o,
        undo_at: None,
        undoes: Some(a.id),
        retry_at: retry.then_some(retry_at),
    };
    core.record(vec![Event::Action(Box::new(rec.clone()))]);
    show(core, &rec);
    // Still in voice with the bot: say that the mute is over.
    if lifted {
        let room = core
            .voice()
            .of_user(a.user)
            .find(|v| v.guild == a.guild)
            .and_then(|v| core.room_of_channel(a.guild, v.channel));
        if let Some(room) = room {
            announce(core, &room, &rec, None);
        }
    }
    !retry
}
