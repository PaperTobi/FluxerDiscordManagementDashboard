//! Audio kept in memory for reuse (rendered speech, decoded clips): the least recently used goes first once the
//! samples take more than the budget. A dropped entry is only rendered or decoded again when it is needed.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::sync::Arc;

/// Bytes of samples each cache keeps (about 11 minutes of 48 kHz audio).
pub const BUDGET_BYTES: usize = 64 << 20;

#[derive(Debug)]
pub struct AudioCache<K> {
    entries: HashMap<K, (Arc<[i16]>, u64)>,
    /// Last use → key, oldest first.
    order: BTreeMap<u64, K>,
    tick: u64,
    bytes: usize,
    budget: usize,
}

impl<K> Default for AudioCache<K> {
    fn default() -> Self {
        AudioCache {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            tick: 0,
            bytes: 0,
            budget: BUDGET_BYTES,
        }
    }
}

impl<K: Clone + Eq + Hash> AudioCache<K> {
    #[cfg(test)]
    fn with_budget(budget: usize) -> Self {
        AudioCache {
            budget,
            ..AudioCache::default()
        }
    }

    pub fn get(&mut self, key: &K) -> Option<Arc<[i16]>> {
        self.tick += 1;
        let (pcm, used) = self.entries.get_mut(key)?;
        self.order.remove(used);
        *used = self.tick;
        self.order.insert(self.tick, key.clone());
        Some(pcm.clone())
    }

    pub fn insert(&mut self, key: K, pcm: Arc<[i16]>) {
        self.remove(&key);
        self.tick += 1;
        self.bytes += pcm.len() * 2;
        self.order.insert(self.tick, key.clone());
        self.entries.insert(key, (pcm, self.tick));
        while self.bytes > self.budget {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some((pcm, _)) = self.entries.remove(&oldest) {
                self.bytes -= pcm.len() * 2;
            }
        }
    }

    pub fn remove(&mut self, key: &K) {
        if let Some((pcm, used)) = self.entries.remove(key) {
            self.order.remove(&used);
            self.bytes -= pcm.len() * 2;
        }
    }

    pub fn clear(&mut self) {
        *self = AudioCache {
            budget: self.budget,
            ..AudioCache::default()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_recently_used_goes_first() {
        let pcm = |n: usize| -> Arc<[i16]> { vec![0; n].into() };
        let mut c = AudioCache::with_budget(100);
        c.insert("a", pcm(20)); // 40 bytes
        c.insert("b", pcm(20));
        assert!(c.get(&"a").is_some()); // a is now the newest
        c.insert("c", pcm(20)); // 120 bytes: b goes
        assert!(c.get(&"b").is_none());
        assert!(c.get(&"a").is_some() && c.get(&"c").is_some());
        c.remove(&"a");
        assert_eq!(c.bytes, 40);
        c.insert("big", pcm(200)); // larger than the whole budget: not kept, and everything older goes
        assert!(c.get(&"big").is_none() && c.get(&"c").is_none());
        assert_eq!(c.bytes, 0);
    }
}
