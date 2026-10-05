//! The bot's read API at `/api/v1` (proposal 0009): tokens with scopes, the read routes and their OpenAPI document.
//! The types a client sees are in `pb-api-proto`.

pub mod v1;

pub use v1::*;
