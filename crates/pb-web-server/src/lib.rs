//! The web server: the pages (Leptos), the live socket, login and setup, forms, uploads and media, behind a host
//! allowlist, origin checks and per-session form tokens.

// Leptos page types nest deeply.
#![recursion_limit = "512"]

mod access;
mod auth;
mod forms;
mod host;
mod hosts;
mod live;
mod login;
mod media;
mod server;
mod setup;
mod system;
mod tls;
mod util;
mod voice;

pub use auth::{Sessions, new_setup_code};
pub use server::{WebConfig, WebParts, WebState, router, serve};
