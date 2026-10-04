use std::collections::VecDeque;

/// PCM addressed by absolute sample index. It keeps everything until told to forget the part before an index, so it
/// holds exactly what is still needed (an open sentence and its pre-roll) instead of a fixed length.
#[derive(Debug, Clone, Default)]
pub struct PcmRing {
    buf: VecDeque<i16>,
    /// Absolute index of `buf[0]`.
    start: u64,
}

impl PcmRing {
    pub fn new(base: u64) -> Self {
        PcmRing {
            buf: VecDeque::new(),
            start: base,
        }
    }

    /// Oldest retained index.
    pub fn start(&self) -> u64 {
        self.start
    }

    /// One past the newest index.
    pub fn end(&self) -> u64 {
        self.start + self.buf.len() as u64
    }

    pub fn append(&mut self, samples: &[i16]) {
        self.buf.extend(samples.iter().copied());
    }

    /// Copy of `[s0, s1)`, clamped to what is retained.
    pub fn slice(&self, s0: u64, s1: u64) -> Vec<i16> {
        let s0 = s0.max(self.start);
        let s1 = s1.min(self.end());
        if s1 <= s0 {
            return Vec::new();
        }
        let a = (s0 - self.start) as usize;
        let b = (s1 - self.start) as usize;
        self.buf.range(a..b).copied().collect()
    }

    /// Forgets everything before `index`.
    pub fn forget_before(&mut self, index: u64) {
        let index = index.min(self.end());
        if index > self.start {
            self.buf.drain(..(index - self.start) as usize);
            self.start = index;
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}
