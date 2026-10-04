//! TLS for every connection the bot opens itself (the Fluxer API and gateway, downloads): rustls with graviola's
//! cryptography (Rust and formally verified assembly, no C), certificates checked against the system's trusted roots
//! (so a private CA works for a self-hosted instance) plus Mozilla's (so a bare container works too).

pub mod v1;

pub use v1::*;
