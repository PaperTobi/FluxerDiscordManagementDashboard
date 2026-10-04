//! Interfaces for everything the bot keeps: the add-only, hash-chained event log; the query index derived from it;
//! content-addressed blobs; the settings, secrets and session files. Implementations live in `pb-store`; the
//! `contract-tests` feature has the tests every implementation must pass.

pub mod v1;

pub use v1::*;
