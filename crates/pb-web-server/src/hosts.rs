//! Only known host names may reach the UI (a defence against DNS rebinding): IP literals, localhost, the Web UI
//! address setting and the extra allowed hosts. Anything else gets 421. A page opened at 0.0.0.0 or [::] (where the bot
//! listens, not an address to open: browsers that reach it run on this machine, and some refuse it) moves to the same
//! page on localhost, so that address never becomes the Web UI address or a login's redirect address.

use std::net::IpAddr;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use http::StatusCode;

use super::server::WebState;

/// The host name of a `Host` header value (without the port; IPv6 literals keep no brackets).
pub fn host_name(header: &str) -> &str {
    if let Some(rest) = header.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    header.rsplit_once(':').map_or(header, |(h, port)| {
        if port.bytes().all(|b| b.is_ascii_digit()) {
            h
        } else {
            header
        }
    })
}

/// Whether a `Host` header value names 0.0.0.0 or [::] (any port).
pub fn unspecified(host: &str) -> bool {
    host_name(host).parse::<IpAddr>().is_ok_and(|ip| ip.is_unspecified())
}

pub fn allowed(state: &WebState, host: &str) -> bool {
    let name = host_name(host).to_ascii_lowercase();
    if name.parse::<IpAddr>().is_ok() || name == "localhost" || name.ends_with(".localhost") {
        return true;
    }
    let eff = state.engine.settings().current().effective(None, None);
    eff.ui_url
        .value
        .as_ref()
        .and_then(|o| o.host())
        .is_some_and(|h| h.eq_ignore_ascii_case(&name))
        || eff
            .allowed_hosts
            .value
            .iter()
            .any(|h| h.as_str().eq_ignore_ascii_case(&name))
}

pub async fn guard(State(state): State<WebState>, req: Request, next: Next) -> Response {
    let host = req
        .headers()
        .get(http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if unspecified(&host) {
        let scheme = if super::auth::https(req.headers()) {
            "https"
        } else {
            "http"
        };
        let port = host.rsplit_once(':').map_or("", |(_, p)| p);
        let port = if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) {
            format!(":{port}")
        } else {
            String::new()
        };
        let path = req.uri().path_and_query().map_or("/", |p| p.as_str());
        return Redirect::temporary(&format!("{scheme}://localhost{port}{path}")).into_response();
    }
    if !allowed(&state, &host) {
        let loc = super::util::locale_of(req.headers());
        return (
            StatusCode::MISDIRECTED_REQUEST,
            pb_i18n::text(loc, "err-unknown-host", &[]),
        )
            .into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names() {
        assert_eq!(host_name("bot.lan:8790"), "bot.lan");
        assert_eq!(host_name("192.168.1.5:8790"), "192.168.1.5");
        assert_eq!(host_name("[::1]:8790"), "::1");
        assert_eq!(host_name("bot.lan"), "bot.lan");
    }

    #[test]
    fn unspecified_addresses() {
        assert!(unspecified("0.0.0.0:8800"));
        assert!(unspecified("0.0.0.0"));
        assert!(unspecified("[::]:8790"));
        assert!(!unspecified("127.0.0.1:8790"));
        assert!(!unspecified("localhost:8790"));
        assert!(!unspecified("bot.lan:8790"));
    }
}
