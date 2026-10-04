//! Every setting the bot has, declared once (`settings!` in `schema.rs`): its type, built-in default, the scopes it
//! may be set at, who may change it and when it takes effect. From that one declaration come the typed per-scope
//! [`Layer`], the resolved [`Effective`] view (each value with where it came from), the exhaustive [`SettingKey`], the
//! metadata the web UI builds its forms from, and the one validator used by settings files, the web API and chat
//! commands. `tree.rs` holds every scope's settings, tracked people and voice lines; `file.rs` reads and edits the
//! TOML files (comments kept).
//!
//! Limits exist only where the model or Fluxer imposes them or a value would be meaningless (a probability outside
//! (0, 1), zero strikes). There are no caps on counts, lengths or durations.

pub mod v1;

pub use v1::*;
