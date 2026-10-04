//! Version 1.

mod builtin;
mod line;
mod pick;
mod plan;
mod resolve;
mod template;

pub use builtin::builtin_text;
pub use line::{Line, LineKey, LineKeyError, Sel, Slot, Slots};
pub use pick::{NoRepeat, SaidTo, pick};
pub use plan::{Part, UtterancePlan, plan};
pub use resolve::{ClipInfo, ClipLang, Resolution, ResolveCtx, ScopedSlots, Source, resolve};
pub use template::{Field, Fields, fill};
