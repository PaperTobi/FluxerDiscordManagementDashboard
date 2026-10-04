//! The Fluxer client: instance discovery, the gateway (identify, heartbeat, resume, reconnect, op 3 and op 4 pacing),
//! REST with Fluxer's rate-limit headers, and the OAuth2 web login. Built to `docs/fluxer-api.md`.

pub mod v1;

pub use v1::*;
