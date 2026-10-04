pub use pb_domain::FRAME;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VadInfo {
    pub model: String,
    /// Samples carried over from the previous frame into the next one.
    pub context: usize,
}

/// Per-stream state of a VAD (opaque to everyone but the model).
#[derive(Debug, Clone, PartialEq)]
pub struct VadState(pub Vec<f32>);

/// Voice-activity detection over 32 ms frames, for many streams at once.
pub trait VadModel: Send + 'static {
    fn info(&self) -> &VadInfo;
    /// A fresh state for a new stream (or after the stream was paused).
    fn new_state(&self) -> VadState;
    /// One frame of each stream in, one speech probability per stream out. `frames[i]` belongs to `states[i]`.
    fn step(&mut self, frames: &[[f32; FRAME]], states: &mut [&mut VadState]) -> Vec<f32>;
}

/// The adaptive noise-floor gate of the old bot (speech = clearly above the quietest recent level), used when a
/// voice-activity model cannot answer. `floor` is its state in dBFS (NaN = none yet). Returns the speech probability.
pub fn energy_gate(floor: &mut f32, frame: &[f32]) -> f32 {
    let ms = frame.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>() / frame.len().max(1) as f64;
    let db = (20.0 * (ms.sqrt() + 1e-9).log10()) as f32;
    if floor.is_nan() || db < *floor {
        *floor = db; // follow quiet moments immediately
    } else {
        *floor += (db - *floor) * 0.002; // drift up slowly so steady noise becomes "floor"
    }
    *floor = floor.clamp(-75.0, -35.0);
    1.0 / (1.0 + (-(db - (*floor + 12.0)) / 3.0).exp())
}
