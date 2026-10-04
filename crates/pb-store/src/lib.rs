//! Storage on the data directory: the hash-chained JSONL event log, content-addressed blobs, the Turso query index
//! derived from the log, and the settings, secrets and session files.

pub mod v1;

pub use v1::*;
