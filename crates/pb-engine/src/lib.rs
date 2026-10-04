//! The bot itself. It follows tracked people into voice, listens to their microphones only, cuts their speech into
//! sentences, scores each sentence, decides (strikes, observe-only, late verdicts, escalation steps), speaks a warning
//! in the person's language, records everything in the event log, and tells the owner and the mod log.
//!
//! Everything external is behind an interface (`pb-fluxer-api`, `pb-voice-api`, `pb-store-api`, the models via
//! `pb-infer`), so the engine runs the same against the real services and against fakes in tests.

pub mod v1;

pub use v1::*;
