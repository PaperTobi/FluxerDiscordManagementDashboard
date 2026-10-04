//! Fluent bundles (de, en) for bot text and the web UI (pure).
//!
//! The `.ftl` files in `locales/` are compiled in. Every message exists in every locale (a test checks), so a
//! missing translation is a build failure, not a surprise in chat.

pub mod v1;

pub use v1::*;
