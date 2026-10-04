//! The model weights and voices the bot ships with, pinned by upstream revision and SHA-256: a manifest, a resumable
//! downloader that checks every file, and a check of what is on disk.

pub mod v1;

pub use v1::*;
