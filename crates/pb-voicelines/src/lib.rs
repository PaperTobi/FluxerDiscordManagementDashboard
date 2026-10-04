//! Everything the bot says out loud is a *voice line*: a warning (per detection type and escalation step), the join
//! greeting, the strike notice, the notices for moderation actions, "say now" presets, and the person's spoken name.
//! Each line has *slots* at global, server and person scope; a slot holds uploaded clips (each tagged with its
//! language) and a text per language that text-to-speech speaks when no clip fits. This crate decides, purely,
//! what is said: which slot, which language, which clip or text, with the placeholders filled in.

pub mod v1;

pub use v1::*;
