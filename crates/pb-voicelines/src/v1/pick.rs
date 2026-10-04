//! Choosing one of several clips: uniformly, never the same one twice in a row for the same person and line.

use std::collections::BTreeMap;

use pb_domain::{BlobHash, GuildId, UserId};

/// Whom a line was said to: a community, a person in it (or nobody in particular), and the line.
pub type SaidTo = (GuildId, Option<UserId>, String);

/// The clip played last per community, person and line, so it is not repeated next time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoRepeat {
    last: BTreeMap<SaidTo, BlobHash>,
}

impl NoRepeat {
    pub fn last(&self, to: &SaidTo) -> Option<&BlobHash> {
        self.last.get(to)
    }

    pub fn remember(&mut self, to: SaidTo, clip: BlobHash) {
        self.last.insert(to, clip);
    }
}

/// Picks a clip with `random` (a number in `0..n` for the given `n`), avoiding `last` when there is a choice.
pub fn pick<'a>(
    candidates: &'a [BlobHash],
    last: Option<&BlobHash>,
    random: &mut dyn FnMut(usize) -> usize,
) -> Option<&'a BlobHash> {
    let pool: Vec<&BlobHash> = if candidates.len() > 1 {
        candidates.iter().filter(|c| Some(*c) != last).collect()
    } else {
        candidates.iter().collect()
    };
    if pool.is_empty() {
        return candidates.first();
    }
    Some(pool[random(pool.len()) % pool.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(n: u8) -> BlobHash {
        BlobHash::from_bytes([n; 32])
    }

    #[test]
    fn never_repeats_when_there_is_a_choice() {
        let c = [h(1), h(2)];
        for r in 0..10 {
            assert_eq!(pick(&c, Some(&h(1)), &mut |_| r), Some(&h(2)));
        }
        assert_eq!(
            pick(&[h(1)], Some(&h(1)), &mut |_| 0),
            Some(&h(1)),
            "a single clip is still played"
        );
        assert_eq!(pick(&[], None, &mut |_| 0), None);
    }
}
