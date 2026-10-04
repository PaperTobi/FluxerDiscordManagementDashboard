//! One-time import of the Python bot's data directory (`bot.db`, recordings, uploaded clips, `secrets.json`, installed
//! voices) into the new storage: settings become TOML (with the removed caps reported), history becomes `sentence`
//! events, recordings and clips become blobs, the old audit log is kept as it was recorded, swear-jar counts and
//! pending timed mutes carry over. Nothing in the old directory is changed or deleted.

pub mod v1;

pub use v1::*;
