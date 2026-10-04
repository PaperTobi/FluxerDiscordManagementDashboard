//! Small helpers the handlers share.

use http::{HeaderMap, header};
use pb_i18n::Locale;

/// The language a request asks for.
pub(crate) fn locale_of(headers: &HeaderMap) -> Locale {
    headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|h| h.to_str().ok())
        .map_or(Locale::En, Locale::negotiate)
}

/// A same-origin check for state-changing requests: an `Origin` header, when the browser sends one, must name this
/// host. (Forms also carry a token; this catches the rest.)
pub(crate) fn same_origin(headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) else {
        return true;
    };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    origin
        .split_once("://")
        .is_some_and(|(_, rest)| rest.eq_ignore_ascii_case(host))
}
