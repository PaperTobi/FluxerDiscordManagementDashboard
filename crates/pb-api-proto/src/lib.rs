//! The bot's read API, as a client sees it: the JSON of every answer, what a token may read, the kinds of events
//! webhooks receive and how their deliveries are signed. A client (like `pb-leaderboard`) needs only this crate.

pub mod v1;
