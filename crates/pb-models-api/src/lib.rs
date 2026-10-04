//! Interfaces for the bot's models: voice-activity detection, the voice-safety classifier and text-to-speech.
//!
//! Each model instance is owned by exactly one thread (the traits require `Send` but deliberately not `Sync`); other
//! parts reach it only by sending jobs to that thread, so a model can never be used from two threads at once.

pub mod v1;

pub use v1::*;
