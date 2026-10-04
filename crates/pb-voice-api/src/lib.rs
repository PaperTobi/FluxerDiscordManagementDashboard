//! The boundary between the bot and the voice system: join a room, receive chosen microphones as 16 kHz PCM, publish
//! one voice track, decide who hears it. Implemented today by `pb-voice-livekit`.

pub mod v1;

pub use v1::*;
