//! Logging in with Fluxer (OAuth2 authorization code with PKCE, scope `identify`) and logging out.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use axum::Form;
use axum::extract::{Query, State};
use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use http::{HeaderMap, header};
use pb_domain::UserId;
use pb_fluxer_api::{Endpoints, OAuthClient, authorize_url};
use pb_i18n::text;
use pb_store_api::{Event, SetupState};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

use super::auth::{
    ADMIN_LIFETIME, OAUTH_COOKIE, OWNER_LIFETIME, SESSION_COOKIE, UserAccess, clear_cookie, cookie, https,
    random_token, record, set_cookie,
};
use super::server::{WebState, redirect_with};
use super::util::locale_of;

/// A pending login can be finished for this long (Fluxer's codes live 10 min as well).
const PENDING_FOR: Duration = Duration::from_secs(600);

/// A login that went to Fluxer and has not come back yet.
pub(crate) struct PendingLogin {
    verifier: String,
    next: String,
    redirect_uri: Url,
    ep: Endpoints,
    /// Started from the setup wizard: finishing it makes this person the owner.
    setup: bool,
    started: Instant,
}

/// A local path to go back to (never another site). Browsers drop tabs and line breaks from `Location` and read `\`
/// as `/`, so a path with any of those (or other control characters or spaces) is refused as a whole.
pub(crate) fn safe_next(next: Option<&str>) -> String {
    match next {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.chars().any(|c| c == '\\' || c.is_whitespace() || c.is_control()) =>
        {
            n.to_owned()
        }
        _ => "/".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::safe_next;

    #[test]
    fn only_local_paths_come_back() {
        for (given, want) in [
            ("/c/1?before=5", "/c/1?before=5"),
            ("//evil.example", "/"),
            ("/\\evil.example", "/"),
            ("/\t/evil.example", "/"),
            ("/\n/evil.example", "/"),
            ("/ /evil.example", "/"),
            ("https://evil.example", "/"),
            ("", "/"),
        ] {
            assert_eq!(safe_next(Some(given)), want, "{given:?}");
        }
    }
}

/// `scheme://host[:port]` as the browser used it.
pub(crate) fn request_origin(headers: &HeaderMap) -> String {
    let scheme = if https(headers) { "https" } else { "http" };
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    format!("{scheme}://{host}")
}

/// The web UI's address: the setting, or the address of this request when none is set.
pub(crate) fn ui_origin(st: &WebState, headers: &HeaderMap) -> String {
    match st.engine.settings().current().effective(None, None).ui_url.value {
        Some(o) => o.to_string(),
        None => request_origin(headers),
    }
}

/// Where Fluxer sends the browser back to (must be registered with the application).
pub(crate) fn redirect_uri(st: &WebState, headers: &HeaderMap) -> String {
    format!("{}/auth/callback", ui_origin(st, headers))
}

/// Whether this person is a bot owner: the application's owner, an extra bot owner, or who finished setup.
pub(crate) fn is_owner(st: &WebState, user: UserId) -> bool {
    st.engine.is_owner(user) || st.secrets.get().setup.owner == Some(user)
}

#[derive(Debug, Deserialize)]
pub struct StartQuery {
    next: Option<String>,
}

/// `GET /login`: off to Fluxer.
pub async fn start(State(st): State<WebState>, headers: HeaderMap, Query(q): Query<StartQuery>) -> Response {
    let loc = locale_of(&headers);
    let secure = https(&headers);
    let in_setup = !st.secrets.get().setup.done;
    if in_setup && st.wizard_session(&headers).is_none() {
        return redirect_with("/setup", vec![]);
    }
    let next = safe_next(q.next.as_deref());
    // The whole login happens on the web UI's address (its cookies are only sent there).
    let origin = ui_origin(&st, &headers);
    if !in_setup && origin != request_origin(&headers) {
        let to = format!(
            "{origin}/login?next={}",
            url::form_urlencoded::byte_serialize(next.as_bytes()).collect::<String>()
        );
        return redirect_with(&to, vec![]);
    }
    let back = if in_setup { "/setup" } else { "/" };
    let fail = |text: String| redirect_with(back, vec![st.notices.put(false, text, secure)]);
    let secrets = st.secrets.get();
    let Some(client_id) = st.secrets.client_id() else {
        return fail(text(
            loc,
            "login-not-ready",
            &[("reason", text(loc, "login-no-token", &[]).into())],
        ));
    };
    let Some(client_secret) = secrets.client_secret else {
        return fail(text(
            loc,
            "login-not-ready",
            &[("reason", text(loc, "login-no-secret", &[]).into())],
        ));
    };
    let ep = match st.engine.login_endpoints().await {
        Ok(ep) => ep,
        Err(e) => return fail(text(loc, "login-unreachable", &[("error", e.into())])),
    };
    let Ok(redirect) = Url::parse(&redirect_uri(&st, &headers)) else {
        return fail(text(
            loc,
            "login-unreachable",
            &[("error", text(loc, "err-bad-ui-address", &[]).into())],
        ));
    };
    // Fluxer only sends people back to a registered address; say so here instead of failing there.
    if let Some(Err(registered)) = st.engine.redirect_registered(redirect.as_str()).await {
        return fail(text(
            loc,
            "login-redirect-not-registered",
            &[
                ("want", redirect.to_string().into()),
                ("registered", registered.join(", ").into()),
            ],
        ));
    }
    let client = OAuthClient {
        client_id,
        client_secret,
        redirect_uri: redirect.clone(),
    };
    let verifier = random_token(32);
    let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_token(24);
    let url = authorize_url(&ep, &client, &state, &challenge);
    st.logins
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            state.clone(),
            PendingLogin {
                verifier,
                next,
                redirect_uri: redirect,
                ep,
                setup: in_setup,
                started: Instant::now(),
            },
        );
    redirect_with(&url, vec![set_cookie(OAUTH_COOKIE, &state, Some(PENDING_FOR), secure)])
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// `GET /auth/callback`: back from Fluxer.
pub async fn callback(State(st): State<WebState>, headers: HeaderMap, Query(q): Query<CallbackQuery>) -> Response {
    let loc = locale_of(&headers);
    let secure = https(&headers);
    let clear = clear_cookie(OAUTH_COOKIE);
    // The state must be the one this browser started with (a login started elsewhere is not finished here).
    let pending = q
        .state
        .as_deref()
        .filter(|s| cookie(&headers, OAUTH_COOKIE) == Some(*s))
        .and_then(|s| {
            st.logins
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(s)
        })
        .filter(|p| p.started.elapsed() < PENDING_FOR);
    let Some(p) = pending else {
        return redirect_with(
            "/",
            vec![clear, st.notices.put(false, text(loc, "login-stale", &[]), secure)],
        );
    };
    let back = if p.setup { "/setup" } else { "/" };
    let fail = |text: String| redirect_with(back, vec![clear.clone(), st.notices.put(false, text, secure)]);
    if let Some(e) = q.error {
        let detail = q.error_description.map_or(e.clone(), |d| format!("{e}: {d}"));
        return fail(text(loc, "login-failed", &[("error", detail.into())]));
    }
    let Some(code) = q.code else {
        return fail(text(
            loc,
            "login-failed",
            &[("error", text(loc, "err-no-login-code", &[]).into())],
        ));
    };
    let (Some(client_id), Some(client_secret)) = (st.secrets.client_id(), st.secrets.get().client_secret) else {
        return fail(text(
            loc,
            "login-not-ready",
            &[("reason", text(loc, "login-no-secret", &[]).into())],
        ));
    };
    let client = OAuthClient {
        client_id,
        client_secret,
        redirect_uri: p.redirect_uri.clone(),
    };
    let user = match st.engine.oauth_user(&p.ep, &client, &code, &p.verifier).await {
        Ok(u) => u,
        Err(e) => return fail(text(loc, "login-failed", &[("error", e.into())])),
    };
    let name = user
        .global_name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| user.username.clone());
    // Finishing setup: whoever holds the setup code and logs in now becomes the owner.
    if p.setup && !st.secrets.get().setup.done && st.wizard_session(&headers).is_some() {
        let done = SetupState {
            done: true,
            owner: Some(user.id),
            finished_at: Some(jiff::Timestamp::now()),
        };
        if let Err(e) = st.secrets.update(|s| s.setup = done).await {
            return fail(pb_web::fmt::store_error(loc, &e));
        }
        if let Err(e) = st.secrets.file.write_setup_code(None).await {
            tracing::warn!(error = %e, "the setup code file could not be removed");
        }
        st.wizard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .finish();
        tracing::info!(user = %user.id, "setup finished");
    }
    let owner = is_owner(&st, user.id);
    let guilds = if owner {
        BTreeSet::new()
    } else {
        st.engine.admin_guilds(user.id).await
    };
    if !owner && guilds.is_empty() {
        return fail(text(loc, "login-no-access", &[("name", name.into())]));
    }
    let life = if owner { OWNER_LIFETIME } else { ADMIN_LIFETIME };
    let rec = record(user.id, name.clone(), user.avatar.clone(), owner);
    let value = st
        .sessions
        .create(
            rec,
            UserAccess {
                owner,
                guilds,
                epoch: 0,
            },
        )
        .await;
    st.engine
        .record(vec![Event::Login(pb_store_api::Login {
            user: user.id,
            name,
            owner,
        })])
        .await;
    let max_age = life.unsigned_abs();
    redirect_with(
        &p.next,
        vec![clear, set_cookie(SESSION_COOKIE, &value, Some(max_age), secure)],
    )
}

#[derive(Debug, Deserialize)]
pub struct LogoutForm {
    #[serde(default)]
    csrf: String,
    /// Ends every login of this person (on every device), not just this one.
    #[serde(default)]
    everywhere: Option<String>,
}

/// `POST /auth/logout`
pub async fn logout(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<LogoutForm>) -> Response {
    match st.sessions.lookup(&headers) {
        Some(s) if st.sessions.csrf_ok(&s.key, &f.csrf) => {
            if f.everywhere.is_some() {
                st.sessions.remove_user(s.record.user).await;
            } else {
                st.sessions.remove(&s.key).await;
            }
            redirect_with("/", vec![clear_cookie(SESSION_COOKIE)])
        }
        Some(_) => {
            let loc = locale_of(&headers);
            redirect_with(
                "/",
                vec![st.notices.put(false, text(loc, "form-expired", &[]), https(&headers))],
            )
        }
        None => redirect_with("/", vec![clear_cookie(SESSION_COOKIE)]),
    }
}

/// Drops logins that never came back.
pub(crate) fn forget_stale(st: &WebState) {
    st.logins
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|_, p| p.started.elapsed() < PENDING_FOR);
}
