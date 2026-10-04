//! A blocking priority queue (highest priority first, then first come first served) with statistics.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::Instant;

struct Entry<T> {
    prio: u8,
    seq: u64,
    at: Instant,
    item: T,
}

impl<T> PartialEq for Entry<T> {
    fn eq(&self, o: &Self) -> bool {
        self.prio == o.prio && self.seq == o.seq
    }
}
impl<T> Eq for Entry<T> {}
impl<T> PartialOrd for Entry<T> {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl<T> Ord for Entry<T> {
    fn cmp(&self, o: &Self) -> Ordering {
        self.prio.cmp(&o.prio).then(o.seq.cmp(&self.seq))
    }
}

struct State<T> {
    heap: BinaryHeap<Entry<T>>,
    seq: u64,
    done: u64,
    closed: bool,
    /// Since when the job handed out last runs (the worker asks for the next one when it is done).
    running: Option<Instant>,
}

/// The queue.
pub struct Queue<T> {
    state: Mutex<State<T>>,
    ready: Condvar,
}

/// A queue's numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct QueueStats {
    pub waiting: u64,
    pub done: u64,
    /// How long the oldest waiting job has waited.
    pub oldest_ms: u64,
    /// How long the job in hand has run (`None`: the worker is idle).
    pub running_ms: Option<u64>,
}

impl<T> std::fmt::Debug for Queue<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Queue").field("stats", &self.stats()).finish()
    }
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Queue {
            state: Mutex::new(State {
                heap: BinaryHeap::new(),
                seq: 0,
                done: 0,
                closed: false,
                running: None,
            }),
            ready: Condvar::new(),
        }
    }
}

impl<T> Queue<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Adds a job (`false` once the queue is closed).
    pub fn push(&self, prio: u8, item: T) -> bool {
        let mut s = self.lock();
        if s.closed {
            return false;
        }
        s.seq += 1;
        let seq = s.seq;
        s.heap.push(Entry {
            prio,
            seq,
            at: Instant::now(),
            item,
        });
        drop(s);
        self.ready.notify_one();
        true
    }

    /// Waits for the next job and how long it waited; `None` once closed and empty.
    pub fn pop(&self) -> Option<(T, std::time::Duration)> {
        let mut s = self.lock();
        s.running = None;
        loop {
            if let Some(e) = s.heap.pop() {
                s.done += 1;
                s.running = Some(Instant::now());
                return Some((e.item, e.at.elapsed()));
            }
            if s.closed {
                return None;
            }
            s = self.ready.wait(s).unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Stops taking jobs; waiting ones are still handed out.
    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }

    pub fn stats(&self) -> QueueStats {
        let s = self.lock();
        let oldest = s.heap.iter().map(|e| e.at.elapsed()).max().unwrap_or_default();
        let ms = |d: std::time::Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        QueueStats {
            waiting: s.heap.len() as u64,
            done: s.done,
            oldest_ms: ms(oldest),
            running_ms: s.running.map(|t| ms(t.elapsed())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highest_priority_first_then_in_order() {
        let q = Queue::default();
        q.push(0, "import");
        q.push(2, "live 1");
        q.push(1, "check");
        q.push(2, "live 2");
        let order: Vec<_> = (0..4).map(|_| q.pop().unwrap().0).collect();
        assert_eq!(order, ["live 1", "live 2", "check", "import"]);
        q.close();
        assert!(q.pop().is_none());
        assert!(!q.push(2, "late"));
        assert_eq!(q.stats().done, 4);
    }
}
