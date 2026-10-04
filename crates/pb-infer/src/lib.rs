//! The models run on their own OS threads and are reached only through jobs: one thread steps the voice-activity
//! model for every stream at once (batched), one runs the classifier (live sentences before re-checks before imports),
//! one speaks (live warnings before previews before pre-rendering). Queues have no length limit; their backlog and
//! the age of the oldest job are reported so the System page can show lag.

pub mod v1;

pub use v1::*;
