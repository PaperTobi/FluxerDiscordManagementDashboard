//! The live-update protocol between the bot and the web UI (JSON over a websocket, `proto: 1`), the reducers both
//! sides use (so a snapshot and the deltas after it can never disagree), the client's per-topic bookkeeping, and the
//! conveyor's stage function.
//!
//! Why the two-tab freeze cannot happen with this protocol: topics are latest-state cells, so a slow or hidden client
//! gets a fresh snapshot instead of a backlog; a hidden tab keeps only the sidebar; every frame carries the client's
//! `view`, so frames meant for an older view are dropped; and the conveyor is a function of timestamps, so a tab that
//! comes back shows the right picture at once instead of replaying what it missed.

pub mod v1;

pub use v1::*;
