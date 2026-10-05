//! The System page's forms (bot owners): replacing the bot token or client secret (with a recent login, or any login
//! while Fluxer rejects the saved ones), reconnecting, reading the settings files again.

use axum::Form;
use axum::extract::State;
use axum::response::Response;
use http::HeaderMap;
use pb_i18n::text;
use secrecy::SecretString;

use super::forms::{Fields, field};
use super::server::WebState;

/// `POST /system/token`
pub async fn token(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.owner_for_credentials(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let loc = s.locale;
    if st.cfg.token_from_env {
        return st.done(&s, &f, false, text(loc, "ui-secret-from-env", &[]));
    }
    let value = field(&f, "value").unwrap_or_default().trim().to_owned();
    match super::setup::use_token(&st, value, loc).await {
        Ok(()) => st.done(&s, &f, true, text(loc, "ui-token-replaced", &[])),
        Err(e) => st.done(&s, &f, false, e),
    }
}

/// `POST /system/client-secret`
pub async fn client_secret(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.owner_for_credentials(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    if st.cfg.client_secret_from_env {
        return st.done(&s, &f, false, text(s.locale, "ui-secret-from-env", &[]));
    }
    let value = field(&f, "value").unwrap_or_default().trim().to_owned();
    if value.is_empty() {
        return st.done(&s, &f, false, text(s.locale, "form-expired", &[]));
    }
    match super::setup::use_client_secret(&st, SecretString::from(value), s.locale).await {
        Ok(()) => st.done(&s, &f, true, text(s.locale, "ui-saved", &[])),
        Err(e) => st.done(&s, &f, false, e),
    }
}

/// `POST /system/reconnect`
pub async fn reconnect(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.owner(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match super::setup::log_in(&st, s.locale).await {
        Ok(()) => st.done(&s, &f, true, text(s.locale, "ui-reconnected", &[])),
        Err(e) => st.done(&s, &f, false, e),
    }
}

/// `POST /system/retry-log`
pub async fn retry_log(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.owner(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match st.engine.retry_log().await {
        Ok(()) => st.done(&s, &f, true, text(s.locale, "ui-log-writing-again", &[])),
        Err(e) => st.done(&s, &f, false, pb_web::fmt::engine_error(s.locale, &e)),
    }
}

/// `POST /system/reload`
pub async fn reload(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.owner(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    match st.engine.reload().await {
        Ok(problems) if problems.is_empty() => st.done(&s, &f, true, text(s.locale, "ui-reloaded", &[])),
        Ok(problems) => {
            let list: Vec<String> = problems.iter().map(ToString::to_string).collect();
            st.done(&s, &f, false, list.join("\n"))
        }
        Err(e) => st.done(&s, &f, false, super::forms::change_error(s.locale, &e)),
    }
}
