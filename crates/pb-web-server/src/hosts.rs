//! Only known host names may reach the UI (a defence against DNS rebinding): IP literals, localhost, the Web UI
//! address setting and the extra allowed hosts. Anything else gets 421.

use std::net::IpAddr;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
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
}
