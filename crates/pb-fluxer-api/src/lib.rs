//! The boundary between the bot and Fluxer. Only `pb-fluxer` speaks Fluxer's protocol; everything else uses these
//! types, so the client can be replaced or faked and Fluxer's own protocol changes stay in one crate.

pub mod v1;

pub use v1::*;
