//! Chat commands (`!pb add @someone`, or a mention of the bot): Fluxer has no slash commands, so commands are plain
//! messages. This crate parses them and says who may run them; the engine carries them out and answers in the
//! community's chat language.

pub mod v1;

pub use v1::*;
