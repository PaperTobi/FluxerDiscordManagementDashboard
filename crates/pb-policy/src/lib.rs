//! The bot's decisions, without I/O or a clock (callers pass `now` in seconds of a monotonic clock): who is in which
//! voice channel ([`VoiceWorld`]), which channels to be in, the voice connection state machine ([`FollowMachine`],
//! a port of the old bot's `follow.py`), and what to do with a flagged sentence ([`Decider`], [`Violations`]).

pub mod v1;

pub use v1::*;
