//! What follows a flagged sentence: the escalation step's action (mute, disconnect, time-out), then the reports with
//! its result. Per person in order (one person's mute is done and reported before their next one), different people
//! at the same time. Fluxer may be slow: nobody waits for this.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use bytes::Bytes;
use pb_domain::{ClfLang, GuildId, UserId};
use pb_settings::EscalationStep;
use pb_store_api::SentenceRecord;
use tokio::sync::oneshot;
use tokio::task::{Id, JoinSet};

use super::core::{Core, RoomHandle};
use super::mailbox::Mailbox;
use super::supervise::{ActorError, Life, Policy, Supervised};

/// The work for one flagged sentence.
#[derive(Debug)]
pub(crate) struct Followup {
    pub sentence: SentenceRecord,
    pub step: Option<EscalationStep>,
    pub room: RoomHandle,
    pub heard: ClfLang,
    /// The sentence's audio (attached to reports when the settings say so).
    pub wav: Bytes,
}

type Person = (GuildId, UserId);

pub(crate) enum EnforcerMsg {
    Follow(Box<Followup>),
    /// Answered once nothing waits or runs any more.
    Drain(oneshot::Sender<()>),
}

pub(crate) struct Enforcer {
    /// Waiting per person (not counting the one being done).
    waiting: HashMap<Person, VecDeque<Followup>>,
    /// Who each running job is for.
    running: HashMap<Id, Person>,
    jobs: JoinSet<Person>,
    drains: Vec<oneshot::Sender<()>>,
}

impl Supervised for Enforcer {
    type Ctx = Arc<Core>;
    type Msg = EnforcerMsg;
    const NAME: &'static str = "enforcer";
    const POLICY: Policy = Policy::Restart;

    async fn start(_: &Arc<Core>) -> Result<Self, ActorError> {
        Ok(Enforcer {
            waiting: HashMap::new(),
            running: HashMap::new(),
            jobs: JoinSet::new(),
            drains: Vec::new(),
        })
    }

    async fn run(mut self, core: Arc<Core>, mb: &mut Mailbox<EnforcerMsg>, life: Life) -> Result<(), ActorError> {
        loop {
            tokio::select! {
                m = mb.recv() => match m {
                    Some(EnforcerMsg::Follow(f)) => self.add(&core, *f),
                    Some(EnforcerMsg::Drain(done)) => self.drains.push(done),
                    None => break,
                },
                Some(done) = self.jobs.join_next_with_id() => self.done(&core, done),
                () = life.cancel.cancelled() => break,
            }
            if self.running.is_empty() {
                for d in self.drains.drain(..) {
                    let _ = d.send(());
                }
            }
        }
        // What runs is finished (an action half done helps nobody); what waits is not started any more.
        let left: usize = self.waiting.values().map(VecDeque::len).sum();
        if left > 0 {
            tracing::warn!(left, "actions and reports not done because the bot stops");
        }
        self.waiting.clear();
        while let Some(done) = self.jobs.join_next_with_id().await {
            self.done(&core, done);
        }
        Ok(())
    }
}

impl Enforcer {
    fn add(&mut self, core: &Arc<Core>, f: Followup) {
        let who = (f.sentence.guild, f.sentence.user);
        if self.running.values().any(|p| *p == who) {
            self.waiting.entry(who).or_default().push_back(f);
        } else {
            self.begin(core, who, f);
        }
    }

    fn begin(&mut self, core: &Arc<Core>, who: Person, f: Followup) {
        let core = core.clone();
        let h = self.jobs.spawn(async move {
            follow_up(&core, f).await;
            who
        });
        self.running.insert(h.id(), who);
    }

    fn done(&mut self, core: &Arc<Core>, done: Result<(Id, Person), tokio::task::JoinError>) {
        let (id, ok) = match done {
            Ok((id, _)) => (id, true),
            Err(e) => (e.id(), false),
        };
        let Some(who) = self.running.remove(&id) else { return };
        if !ok {
            tracing::error!(guild = %who.0, user = %who.1, "an action or report for a flagged sentence failed inside");
        }
        let next = self.waiting.get_mut(&who).and_then(VecDeque::pop_front);
        if self.waiting.get(&who).is_some_and(VecDeque::is_empty) {
            self.waiting.remove(&who);
        }
        if let Some(f) = next {
            self.begin(core, who, f);
        }
    }
}

/// The step's action, then one report with its result.
async fn follow_up(core: &Arc<Core>, f: Followup) {
    let action = match f.step.as_ref().and_then(|st| st.action.kind().map(|k| (st, k))) {
        Some((st, kind)) => Some(super::actions::step_action(core, &f.sentence, kind, st, f.room, Some(f.heard)).await),
        None => None,
    };
    super::reports::flagged(core, &f.sentence, f.step.as_ref(), action.as_ref(), &f.wav).await;
}
