//! The bot side of live updates: each topic is a cell holding its latest state, changed only by deltas that are also
//! broadcast; a subscriber gets a consistent snapshot plus the deltas after it, and a subscriber that falls behind gets
//! a fresh snapshot instead of a backlog.

pub mod v1;

pub use v1::*;
