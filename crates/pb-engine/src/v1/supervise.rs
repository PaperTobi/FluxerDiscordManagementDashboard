//! Supervision of the engine's long-lived parts. Each runs as a [`Supervised`] actor under the [`Supervisor`]: one that panics
//! or fails is started again after a pause, with its state built afresh and its mailbox kept (only the message being
//! handled is lost); one whose state cannot be rebuilt, or that keeps failing, is fatal and the bot stops, so the
//! service manager starts it again.

use std::collections::VecDeque;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::FutureExt;
use tokio::sync::watch;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::core::lock;
use super::health::{ActorHealth, ActorState, EngineHealth, FatalError};
use super::mailbox::{Activity, Mailbox, mailbox};

/// The first pause before a crashed actor starts again; it doubles with every crash up to [`MAX_PAUSE`].
const FIRST_PAUSE: Duration = Duration::from_millis(100);
const MAX_PAUSE: Duration = Duration::from_secs(30);
/// Running this long without a crash starts the pauses over.
const HEALTHY_AFTER: Duration = Duration::from_secs(60);
/// More crashes than this within [`CRASH_WINDOW`] are fatal.
const MAX_CRASHES: usize = 5;
const CRASH_WINDOW: Duration = Duration::from_secs(600);
/// Messages waiting this long without the actor taking one: it does not answer.
const NOT_ANSWERING_MS: u64 = 60_000;

/// Why an actor could not go on.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ActorError {
    #[error(transparent)]
    Store(#[from] pb_store_api::StoreError),
}

/// What happens when an actor crashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Policy {
    /// It starts again after a pause.
    Restart,
    /// Its state cannot be rebuilt: the bot stops.
    Fatal,
}

/// What a running actor gets besides its mailbox.
#[derive(Debug, Clone)]
pub(crate) struct Life {
    /// Cancelled when the engine shuts down.
    pub cancel: CancellationToken,
    activity: Arc<Activity>,
}

impl Life {
    /// Shows the actor is alive (actors that work on timers; taking a message counts by itself).
    pub fn beat(&self) {
        self.activity.beat();
    }
}

/// A long-lived part of the engine.
pub(crate) trait Supervised: Sized + Send + 'static {
    /// What it works with (the engine's core).
    type Ctx: Clone + Send + Sync + 'static;
    /// What others send it (`Infallible` when nobody does).
    type Msg: Send + 'static;
    const NAME: &'static str;
    const POLICY: Policy;

    /// Builds its state: at the first start and after every crash (from the index where needed).
    fn start(ctx: &Self::Ctx) -> impl Future<Output = Result<Self, ActorError>> + Send;

    /// Runs until the mailbox closes or `life.cancel` is cancelled.
    fn run(
        self,
        ctx: Self::Ctx,
        mb: &mut Mailbox<Self::Msg>,
        life: Life,
    ) -> impl Future<Output = Result<(), ActorError>> + Send;
}

#[derive(Debug)]
struct Slot {
    name: &'static str,
    activity: Arc<Activity>,
    status: Mutex<Status>,
    /// Cancelled once the actor ended for good.
    ended: CancellationToken,
}

#[derive(Debug)]
struct Status {
    state: ActorState,
    restarts: u32,
    last_error: Option<String>,
}

impl Slot {
    fn set(&self, state: ActorState) {
        lock(&self.status).state = state;
    }

    fn crashed(&self, state: ActorState, error: String) {
        let mut s = lock(&self.status);
        s.state = state;
        s.last_error = Some(error);
        if matches!(state, ActorState::Restarting { .. }) {
            s.restarts += 1;
        }
    }
}

/// Runs the engine's actors and knows how they are doing.
#[derive(Debug)]
pub(crate) struct Supervisor {
    tracker: TaskTracker,
    cancel: CancellationToken,
    slots: Mutex<Vec<Arc<Slot>>>,
    fatal: watch::Sender<Option<FatalError>>,
}

impl Default for Supervisor {
    fn default() -> Self {
        Supervisor {
            tracker: TaskTracker::new(),
            cancel: CancellationToken::new(),
            slots: Mutex::new(Vec::new()),
            fatal: watch::Sender::new(None),
        }
    }
}

impl Supervisor {
    /// Starts an actor with the mailbox others already send to.
    pub fn spawn<A: Supervised>(&self, ctx: A::Ctx, mb: Mailbox<A::Msg>) {
        let slot = Arc::new(Slot {
            name: A::NAME,
            activity: mb.activity(),
            status: Mutex::new(Status {
                state: ActorState::Running,
                restarts: 0,
                last_error: None,
            }),
            ended: CancellationToken::new(),
        });
        lock(&self.slots).push(slot.clone());
        let cancel = self.cancel.child_token();
        let fatal = self.fatal.clone();
        self.tracker.spawn(keep_running::<A>(ctx, mb, slot, cancel, fatal));
    }

    /// Starts an actor nobody sends messages to.
    pub fn spawn_alone<A: Supervised>(&self, ctx: A::Ctx) {
        let (_, mb) = mailbox();
        self.spawn::<A>(ctx, mb);
    }

    pub fn health(&self) -> EngineHealth {
        let actors = lock(&self.slots)
            .iter()
            .map(|s| {
                let st = lock(&s.status);
                let (queued, idle_ms) = (s.activity.queued(), s.activity.idle_ms());
                let state = if st.state == ActorState::Running && queued > 0 && idle_ms > NOT_ANSWERING_MS {
                    ActorState::NotAnswering
                } else {
                    st.state
                };
                ActorHealth {
                    name: s.name,
                    state,
                    restarts: st.restarts,
                    queued,
                    idle_ms,
                    last_error: st.last_error.clone(),
                }
            })
            .collect();
        EngineHealth {
            actors,
            fatal: self.fatal.borrow().clone(),
        }
    }

    /// Waits until an actor failed for good.
    pub async fn fatal(&self) -> FatalError {
        let mut rx = self.fatal.subscribe();
        match rx.wait_for(Option::is_some).await {
            Ok(f) => f.clone().unwrap_or_else(|| unreachable!("waited for Some")),
            // The supervisor holds the sender: this does not happen while it exists.
            Err(_) => std::future::pending().await,
        }
    }

    /// Waits until the actor called `name` ended for good (at once if there is none).
    pub async fn ended(&self, name: &str) {
        let ended = lock(&self.slots)
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.ended.clone());
        if let Some(e) = ended {
            e.cancelled().await;
        }
    }

    /// Cancels every actor and waits (at most `limit`) until they ended.
    pub async fn stop(&self, limit: Duration) {
        self.cancel.cancel();
        self.tracker.close();
        if tokio::time::timeout(limit, self.tracker.wait()).await.is_err() {
            tracing::warn!("some parts of the engine did not stop in time");
        }
    }
}

async fn keep_running<A: Supervised>(
    ctx: A::Ctx,
    mut mb: Mailbox<A::Msg>,
    slot: Arc<Slot>,
    cancel: CancellationToken,
    fatal: watch::Sender<Option<FatalError>>,
) {
    let _ended = slot.ended.clone().drop_guard();
    let mut crashes: VecDeque<Instant> = VecDeque::new();
    let mut pause = FIRST_PAUSE;
    loop {
        slot.set(ActorState::Running);
        let began = Instant::now();
        let life = Life {
            cancel: cancel.clone(),
            activity: slot.activity.clone(),
        };
        let ran = AssertUnwindSafe(async {
            let actor = A::start(&ctx).await?;
            actor.run(ctx.clone(), &mut mb, life).await
        })
        .catch_unwind()
        .await;
        let error = match ran {
            Ok(Ok(())) => {
                slot.set(ActorState::Stopped);
                return;
            }
            Ok(Err(e)) => e.to_string(),
            Err(panic) => panic_message(panic.as_ref()),
        };
        if cancel.is_cancelled() {
            slot.set(ActorState::Stopped);
            return;
        }
        let now = Instant::now();
        if now.duration_since(began) > HEALTHY_AFTER {
            pause = FIRST_PAUSE;
        }
        crashes.push_back(now);
        crashes.retain(|t| now.duration_since(*t) < CRASH_WINDOW);
        if A::POLICY == Policy::Fatal || crashes.len() > MAX_CRASHES {
            tracing::error!(actor = A::NAME, %error, "a part of the engine failed for good; the bot stops");
            slot.crashed(ActorState::Failed, error.clone());
            fatal.send_if_modified(|f| {
                let first = f.is_none();
                if first {
                    *f = Some(FatalError { actor: A::NAME, error });
                }
                first
            });
            return;
        }
        let attempt = u32::try_from(crashes.len()).unwrap_or(u32::MAX);
        tracing::error!(actor = A::NAME, %error, attempt, "a part of the engine crashed; starting it again in {pause:?}");
        slot.crashed(ActorState::Restarting { attempt }, error);
        tokio::select! {
            () = tokio::time::sleep(pause) => {}
            () = cancel.cancelled() => {
                slot.set(ActorState::Stopped);
                return;
            }
        }
        pause = (pause * 2).min(MAX_PAUSE);
    }
}

/// A panic's message.
fn panic_message(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>()
        .map(|s| (*s).to_owned())
        .or_else(|| p.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a panic without a message".to_owned())
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::v1::mailbox::Addr;

    /// Counts what it gets; panics on 0; ends on `u32::MAX`.
    struct Counter(Arc<AtomicU32>);

    impl Supervised for Counter {
        type Ctx = Arc<AtomicU32>;
        type Msg = u32;
        const NAME: &'static str = "counter";
        const POLICY: Policy = Policy::Restart;

        async fn start(ctx: &Self::Ctx) -> Result<Self, ActorError> {
            Ok(Counter(ctx.clone()))
        }

        async fn run(self, _: Self::Ctx, mb: &mut Mailbox<u32>, _: Life) -> Result<(), ActorError> {
            while let Some(n) = mb.recv().await {
                assert_ne!(n, 0, "zero");
                if n == u32::MAX {
                    return Ok(());
                }
                self.0.fetch_add(n, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    struct Doomed;

    impl Supervised for Doomed {
        type Ctx = ();
        type Msg = Infallible;
        const NAME: &'static str = "doomed";
        const POLICY: Policy = Policy::Fatal;

        async fn start((): &()) -> Result<Self, ActorError> {
            Ok(Doomed)
        }

        async fn run(self, (): (), _: &mut Mailbox<Infallible>, _: Life) -> Result<(), ActorError> {
            panic!("broken")
        }
    }

    async fn settle(sup: &Supervisor, name: &str, want: impl Fn(&ActorHealth) -> bool) -> ActorHealth {
        loop {
            if let Some(a) = sup.health().actors.into_iter().find(|a| a.name == name && want(a)) {
                return a;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn counter(sup: &Supervisor) -> (Addr<u32>, Arc<AtomicU32>) {
        let (addr, mb) = mailbox();
        let total = Arc::new(AtomicU32::new(0));
        sup.spawn::<Counter>(total.clone(), mb);
        (addr, total)
    }

    #[tokio::test(start_paused = true)]
    async fn a_crash_restarts_with_the_mailbox_kept() {
        let sup = Supervisor::default();
        let (addr, total) = counter(&sup);
        for n in [1, 0, 2, 3] {
            addr.send(n).unwrap();
        }
        let a = settle(&sup, "counter", |a| {
            a.restarts == 1 && a.state == ActorState::Running && a.queued == 0
        })
        .await;
        assert_eq!(
            a.last_error.as_deref(),
            Some("assertion `left != right` failed: zero\n  left: 0\n right: 0")
        );
        assert_eq!(
            total.load(Ordering::SeqCst),
            6,
            "only the message being handled was lost"
        );
        assert!(!sup.health().failing() && sup.health().fatal.is_none());
        addr.send(u32::MAX).unwrap();
        settle(&sup, "counter", |a| a.state == ActorState::Stopped).await;
    }

    #[tokio::test(start_paused = true)]
    async fn crashing_again_and_again_is_fatal() {
        let sup = Supervisor::default();
        let (addr, _) = counter(&sup);
        for _ in 0..=MAX_CRASHES {
            addr.send(0).unwrap();
        }
        let f = sup.fatal().await;
        assert_eq!(f.actor, "counter");
        assert_eq!(settle(&sup, "counter", |_| true).await.state, ActorState::Failed);
        assert!(sup.health().failing());
    }

    #[tokio::test(start_paused = true)]
    async fn a_fatal_actor_stops_the_bot_at_once() {
        let sup = Supervisor::default();
        sup.spawn_alone::<Doomed>(());
        assert_eq!(sup.fatal().await.error, "broken");
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_messages_without_progress_mean_not_answering() {
        struct Stuck;
        impl Supervised for Stuck {
            type Ctx = ();
            type Msg = ();
            const NAME: &'static str = "stuck";
            const POLICY: Policy = Policy::Restart;
            async fn start((): &()) -> Result<Self, ActorError> {
                Ok(Stuck)
            }
            async fn run(self, (): (), _: &mut Mailbox<()>, life: Life) -> Result<(), ActorError> {
                life.cancel.cancelled().await;
                Ok(())
            }
        }
        let sup = Supervisor::default();
        let (addr, mb) = mailbox();
        sup.spawn::<Stuck>((), mb);
        addr.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(NOT_ANSWERING_MS + 1000)).await;
        assert_eq!(settle(&sup, "stuck", |_| true).await.state, ActorState::NotAnswering);
        sup.stop(Duration::from_secs(1)).await;
        assert_eq!(settle(&sup, "stuck", |_| true).await.state, ActorState::Stopped);
    }
}
