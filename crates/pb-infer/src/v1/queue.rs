//! A blocking job queue with statistics. The highest priority goes first. Within a priority, jobs with a deadline go
//! earliest deadline first, then jobs without one in order, then jobs whose deadline has passed (they are still done,
//! never dropped, and get every fourth turn while others wait, so they always move on). Jobs of one flow (one person)
//! are handed out in the order they came, whatever their deadlines.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

struct Entry<T> {
    prio: u8,
    seq: u64,
    at: Instant,
    deadline: Option<Instant>,
    flow: Option<u64>,
    item: T,
}

/// A job's place in the order (smaller goes first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    OnTime(Instant),
    Whenever,
    Late,
}

struct State<T> {
    waiting: Vec<Entry<T>>,
    seq: u64,
    done: u64,
    late: u64,
    /// Turns handed out while late jobs waited behind others (every fourth goes to a late job).
    turns: u64,
    closed: bool,
    /// Since when the job handed out last runs (the worker asks for the next one when it is done).
    running: Option<Instant>,
}

impl<T> State<T> {
    /// The job to hand out next.
    fn pick(&mut self, now: Instant) -> Option<usize> {
        // Only the oldest job of each flow may go.
        let mut heads: HashMap<u64, u64> = HashMap::new();
        for e in &self.waiting {
            if let Some(f) = e.flow {
                let h = heads.entry(f).or_insert(e.seq);
                *h = (*h).min(e.seq);
            }
        }
        let free = |e: &Entry<T>| e.flow.is_none_or(|f| heads.get(&f) == Some(&e.seq));
        let top = self.waiting.iter().filter(|e| free(e)).map(|e| e.prio).max()?;
        let class = |e: &Entry<T>| match e.deadline {
            Some(d) if d > now => Class::OnTime(d),
            Some(_) => Class::Late,
            None => Class::Whenever,
        };
        let candidates = || {
            self.waiting
                .iter()
                .enumerate()
                .filter(|(_, e)| e.prio == top && free(e))
        };
        let best = candidates().min_by_key(|(_, e)| (class(e), e.seq)).map(|(i, _)| i)?;
        if class(&self.waiting[best]) == Class::Late {
            return Some(best);
        }
        let oldest_late = candidates()
            .filter(|(_, e)| class(e) == Class::Late)
            .min_by_key(|(_, e)| e.seq)
            .map(|(i, _)| i);
        match oldest_late {
            Some(late) => {
                self.turns += 1;
                Some(if self.turns.is_multiple_of(4) { late } else { best })
            }
            None => Some(best),
        }
    }
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
    /// Jobs handed out after their deadline.
    pub late: u64,
    /// How long the oldest waiting job has waited.
    pub oldest_ms: u64,
    /// How long the job in hand has run (`None`: the worker is idle).
    pub running_ms: Option<u64>,
}

impl QueueStats {
    /// Two queues' numbers as one (the longest waits).
    pub fn merge(self, o: QueueStats) -> QueueStats {
        QueueStats {
            waiting: self.waiting + o.waiting,
            done: self.done + o.done,
            late: self.late + o.late,
            oldest_ms: self.oldest_ms.max(o.oldest_ms),
            running_ms: self.running_ms.max(o.running_ms),
        }
    }
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
                waiting: Vec::new(),
                seq: 0,
                done: 0,
                late: 0,
                turns: 0,
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

    /// Adds a job without a deadline or flow (`false` once the queue is closed).
    pub fn push(&self, prio: u8, item: T) -> bool {
        self.push_job(prio, None, None, item)
    }

    /// Adds a job (`false` once the queue is closed).
    pub fn push_job(&self, prio: u8, deadline: Option<Instant>, flow: Option<u64>, item: T) -> bool {
        let mut s = self.lock();
        if s.closed {
            return false;
        }
        s.seq += 1;
        let seq = s.seq;
        s.waiting.push(Entry {
            prio,
            seq,
            at: Instant::now(),
            deadline,
            flow,
            item,
        });
        drop(s);
        self.ready.notify_one();
        true
    }

    /// Waits for the next job and how long it waited; `None` once closed and empty.
    pub fn pop(&self) -> Option<(T, Duration)> {
        let mut s = self.lock();
        s.running = None;
        loop {
            let now = Instant::now();
            if let Some(i) = s.pick(now) {
                let e = s.waiting.swap_remove(i);
                s.done += 1;
                if e.deadline.is_some_and(|d| d <= now) {
                    s.late += 1;
                }
                s.running = Some(now);
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
        let oldest = s.waiting.iter().map(|e| e.at.elapsed()).max().unwrap_or_default();
        let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
        QueueStats {
            waiting: s.waiting.len() as u64,
            done: s.done,
            late: s.late,
            oldest_ms: ms(oldest),
            running_ms: s.running.map(|t| ms(t.elapsed())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order<T: Copy>(q: &Queue<T>, n: usize) -> Vec<T> {
        (0..n).map(|_| q.pop().unwrap().0).collect()
    }

    #[test]
    fn highest_priority_first_then_in_order() {
        let q = Queue::default();
        q.push(0, "import");
        q.push(2, "live 1");
        q.push(1, "check");
        q.push(2, "live 2");
        assert_eq!(order(&q, 4), ["live 1", "live 2", "check", "import"]);
        q.close();
        assert!(q.pop().is_none());
        assert!(!q.push(2, "late"));
        assert_eq!(q.stats().done, 4);
    }

    #[test]
    fn earliest_deadline_first_then_without_then_late() {
        let now = Instant::now();
        let q = Queue::default();
        q.push_job(2, None, None, "whenever");
        q.push_job(2, Some(now + Duration::from_secs(9)), None, "in 9 s");
        q.push_job(2, Some(now - Duration::from_secs(1)), None, "late");
        q.push_job(2, Some(now + Duration::from_secs(3)), None, "in 3 s");
        assert_eq!(order(&q, 4), ["in 3 s", "in 9 s", "whenever", "late"]);
        assert_eq!(q.stats().late, 1);
    }

    #[test]
    fn late_jobs_get_every_fourth_turn() {
        let now = Instant::now();
        let q = Queue::default();
        q.push_job(2, Some(now - Duration::from_secs(1)), None, 0);
        for i in 1..=6 {
            q.push_job(2, Some(now + Duration::from_secs(60)), None, i);
        }
        assert_eq!(order(&q, 7), [1, 2, 3, 0, 4, 5, 6]);
    }

    #[test]
    fn one_flow_keeps_its_order() {
        let now = Instant::now();
        let q = Queue::default();
        // Ada's first sentence is already late, her second is not: hers still go in order; Max's goes first.
        q.push_job(2, Some(now - Duration::from_secs(1)), Some(1), "ada 1");
        q.push_job(2, Some(now + Duration::from_secs(5)), Some(1), "ada 2");
        q.push_job(2, Some(now + Duration::from_secs(9)), Some(2), "max 1");
        assert_eq!(order(&q, 3), ["max 1", "ada 1", "ada 2"]);
    }
}
