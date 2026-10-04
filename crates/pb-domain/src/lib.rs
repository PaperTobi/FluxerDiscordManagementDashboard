//! Domain types shared by every part of the bot: Fluxer ids, the classifier's labels and languages, language tags,
//! settings scopes, who hears the bot, why it speaks, moderation actions and their outcomes, voice states, the audio
//! formats, blob hashes and sentence ids. Pure data: no I/O, no async, compiles for the browser.

pub mod v1;

pub use v1::*;
