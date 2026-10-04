use super::FRAME;

/// Cuts a stream of samples into 512-sample frames (`FrameAssembler.push_samples`).
#[derive(Debug, Clone, Default)]
pub struct FrameAssembler {
    rest: Vec<i16>,
}

impl FrameAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends samples and returns the complete frames now available.
    pub fn push(&mut self, samples: &[i16]) -> Vec<[i16; FRAME]> {
        self.rest.extend_from_slice(samples);
        let (frames, _) = self.rest.as_chunks::<FRAME>();
        let out = frames.to_vec();
        self.rest.drain(..out.len() * FRAME);
        out
    }
}
