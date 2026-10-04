//! Mailboxes of the engine's actors: unbounded channels (nothing is ever refused) that note how much waits in them
//! and when their actor last took something out, for the health check.

use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::sync::mpsc;

/// The engine's own time base for activity stamps (tokio time, so tests with paused time move it).
static EPOCH: LazyLock<tokio::time::Instant> = LazyLock::new(tokio::time::Instant::now);

pub(crate) fn now_ms() -> u64 {
    u64::try_from(EPOCH.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// What a mailbox has seen, and when its actor last showed it is alive.
#[derive(Debug)]
pub(crate) struct Activity {
    sent: AtomicU64,
    received: AtomicU64,
    last_ms: AtomicU64,
}

impl Default for Activity {
    fn default() -> Self {
        Activity {
            sent: AtomicU64::new(0),
            received: AtomicU64::new(0),
            last_ms: AtomicU64::new(now_ms()),
        }
    }
}

impl Activity {
    /// Messages sent and not taken out yet.
    pub fn queued(&self) -> u64 {
        self.sent
            .load(Ordering::Relaxed)
            .saturating_sub(self.received.load(Ordering::Relaxed))
    }

    /// The actor is alive (it took a message, or a timer of its own fired).
    pub fn beat(&self) {
        self.last_ms.store(now_ms(), Ordering::Relaxed);
    }

    /// Milliseconds since the actor last showed it is alive.
    pub fn idle_ms(&self) -> u64 {
        now_ms().saturating_sub(self.last_ms.load(Ordering::Relaxed))
    }
}

/// A new mailbox and the address to send to it.
pub(crate) fn mailbox<T>() -> (Addr<T>, Mailbox<T>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let activity = Arc::new(Activity::default());
    (
        Addr {
            tx,
            activity: activity.clone(),
        },
        Mailbox { rx, activity },
    )
}

/// Where to send an actor messages (cheap to clone).
#[derive(Debug)]
pub(crate) struct Addr<T> {
    tx: mpsc::UnboundedSender<T>,
    activity: Arc<Activity>,
}

impl<T> Clone for Addr<T> {
    fn clone(&self) -> Self {
        Addr {
            tx: self.tx.clone(),
            activity: self.activity.clone(),
        }
    }
}

impl<T> Addr<T> {
    /// Sends; the message comes back when the actor is gone for good.
    pub fn send(&self, m: T) -> Result<(), mpsc::error::SendError<T>> {
        self.activity.sent.fetch_add(1, Ordering::Relaxed);
        self.tx.send(m).inspect_err(|_| {
            self.activity.sent.fetch_sub(1, Ordering::Relaxed);
        })
    }
}

/// An actor's incoming messages. It belongs to the actor's supervisor, so it outlives a crash of the actor: only the
/// message being handled is lost.
#[derive(Debug)]
pub(crate) struct Mailbox<T> {
    rx: mpsc::UnboundedReceiver<T>,
    activity: Arc<Activity>,
}

impl<T> Mailbox<T> {
    /// The next message (`None` once every address is gone). Safe to use in `select!`.
    pub async fn recv(&mut self) -> Option<T> {
        let m = self.rx.recv().await;
        self.took(m.is_some());
        m
    }

    fn took(&self, some: bool) {
        if some {
            self.activity.received.fetch_add(1, Ordering::Relaxed);
        }
        self.activity.beat();
    }

    pub(crate) fn activity(&self) -> Arc<Activity> {
        self.activity.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn counts_what_waits() {
        let (addr, mut mb) = mailbox::<u32>();
        addr.send(1).unwrap();
        addr.send(2).unwrap();
        assert_eq!(mb.activity().queued(), 2);
        assert_eq!(mb.recv().await, Some(1));
        assert_eq!(mb.activity().queued(), 1);
        assert_eq!(mb.recv().await, Some(2));
        assert_eq!(mb.activity().queued(), 0);
        drop(addr);
        assert_eq!(mb.recv().await, None);
    }
}
