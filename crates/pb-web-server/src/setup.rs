//! The setup wizard's steps (the page is `pb_web::pages::setup`). The setup code proves access to the server; it
//! opens a wizard session (1 h) in which the instance, the bot token and the client secret are set, and ends with an
//! owner login.
//!
//! The wizard opens again after setup when Fluxer no longer accepts the bot token or the client secret (both reset in
//! Fluxer, say): nobody can log in then, so a new code from the bot's log lets the token and the secret be replaced
//! (the owner and every setting stay).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use axum::Form;
use axum::extract::{ConnectInfo, State};
use axum::response::Response;
use http::{HeaderMap, HeaderValue};
use pb_domain::Scope;
use pb_engine::Connection;
use pb_i18n::{Locale, text};
use pb_settings::{InstanceUrl, SettingKey};
use pb_store_api::{Actor, Via};
use pb_web::app::{SetupStep, SetupView};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::auth::{SETUP_COOKIE, code_matches, cookie, https, new_setup_code, random_token, set_cookie, sha256_hex};
use super::login::{redirect_uri, request_origin};
use super::server::{WebState, redirect_with};
use super::util::{locale_of, same_origin};

const SESSION_FOR: Duration = Duration::from_secs(3600);
/// How long the token step waits for the bot to log in with a new token.
const LOGIN_WAIT: Duration = Duration::from_secs(20);

struct WizardSession {
    started: Instant,
    instance_ok: bool,
    /// A done step opened again (shown until it is saved again or kept as it is).
    revisit: Option<SetupStep>,
    /// The instance address last typed that did not work (shown again instead of the saved one).
    typed_instance: Option<String>,
}

struct Guess {
    fails: u32,
    until: Instant,
}

/// The wizard's state (in memory: a restart asks for a new code).
#[derive(Default)]
pub(crate) struct Wizard {
    pub code: Option<String>,
    sessions: HashMap<String, WizardSession>,
    guesses: HashMap<IpAddr, Guess>,
}

impl Wizard {
    pub fn finish(&mut self) {
        self.code = None;
        self.sessions.clear();
        self.guesses.clear();
    }

    /// How long `ip` must wait before trying another code.
    fn wait(&self, ip: Option<IpAddr>) -> Duration {
        ip.and_then(|ip| self.guesses.get(&ip))
            .map_or(Duration::ZERO, |g| g.until.saturating_duration_since(Instant::now()))
    }
}

/// Wrong codes from one address: 1 s, 2 s, 4 s … up to about 4 min between tries.
fn delay(fails: u32) -> Duration {
    Duration::from_secs(1u64 << fails.saturating_sub(1).min(8))
}

impl WebState {
    fn wizard(&self) -> std::sync::MutexGuard<'_, Wizard> {
        self.wizard.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The key of this browser's wizard session, if it has a valid one.
    pub(crate) fn wizard_session(&self, headers: &HeaderMap) -> Option<String> {
        let id = self.sessions.signer().open("setup", cookie(headers, SETUP_COOKIE)?)?;
        let key = sha256_hex(id);
        let w = self.wizard();
        w.sessions
            .get(&key)
            .filter(|s| s.started.elapsed() < SESSION_FOR)
            .map(|_| key)
    }

    /// Fluxer rejects the saved bot token (or there is none), or refused the client secret at the last login.
    pub(crate) fn credentials_broken(&self) -> bool {
        matches!(
            self.engine.connection(),
            Connection::TokenRejected | Connection::NoToken
        ) || self.client_rejected.load(Ordering::Relaxed)
    }

    /// Setup is finished but Fluxer no longer accepts the bot's credentials: the wizard is open again to replace them.
    pub(crate) fn repairing(&self) -> bool {
        self.secrets.get().setup.done && self.credentials_broken()
    }

    /// Makes a code while the wizard is open (setup unfinished, or repairing) and drops it once it is not.
    pub(crate) async fn refresh_code(&self) {
        let open = !self.secrets.get().setup.done || self.repairing();
        let (made, dropped) = {
            let mut w = self.wizard();
            match (open, w.code.is_some()) {
                (true, false) => {
                    let code = new_setup_code();
                    w.code = Some(code.clone());
                    (Some(code), false)
                }
                (false, true) => {
                    w.finish();
                    (None, true)
                }
                _ => (None, false),
            }
        };
        if let Some(code) = made {
            if let Err(e) = self.secrets.file.write_setup_code(Some(&code)).await {
                tracing::warn!(error = %e, "the setup code file could not be written");
            }
            tracing::warn!(
                "Fluxer does not accept the bot token or the client secret: open /setup in the web UI and enter the \
                 code {code} (it is also in {}) to replace them",
                self.cfg.setup_code_file
            );
        } else if dropped && let Err(e) = self.secrets.file.write_setup_code(None).await {
            tracing::warn!(error = %e, "the setup code file could not be removed");
        }
    }

    pub(crate) fn setup_view(&self, headers: &HeaderMap, ip: Option<IpAddr>) -> SetupView {
        let secrets = self.secrets.get();
        let key = self.wizard_session(headers);
        let (instance_ok, revisit, typed_instance, wait) = {
            let w = self.wizard();
            let session = key.as_ref().and_then(|k| w.sessions.get(k));
            (
                session.is_some_and(|s| s.instance_ok),
                session.and_then(|s| s.revisit),
                session.and_then(|s| s.typed_instance.clone()),
                w.wait(ip),
            )
        };
        let repair = secrets.setup.done && self.credentials_broken();
        let reached = if secrets.setup.done && !repair {
            SetupStep::Done
        } else if key.is_none() {
            SetupStep::Code
        } else if !instance_ok {
            SetupStep::Instance
        } else if secrets.bot_token.is_none()
            || matches!(
                self.engine.connection(),
                Connection::TokenRejected | Connection::NoToken
            )
        {
            SetupStep::Token
        } else if secrets.client_secret.is_none() || self.client_rejected.load(Ordering::Relaxed) {
            SetupStep::ClientSecret
        } else {
            SetupStep::Owner
        };
        let step = revisit.filter(|r| *r < reached).unwrap_or(reached);
        let eff = self.engine.settings().current().effective(None, None);
        SetupView {
            step,
            reached,
            csrf: key
                .map(|k| self.sessions.signer().tag("setup-csrf", &k))
                .unwrap_or_default(),
            instance: eff.instance.value.to_string(),
            typed_instance,
            bot: self
                .engine
                .identity()
                .map(|i| i.user.global_name.clone().unwrap_or(i.user.username.clone())),
            redirect_uri: redirect_uri(self, headers),
            wait_secs: wait.as_secs() + u64::from(wait.subsec_nanos() > 0),
            code_file: self.cfg.setup_code_file.clone(),
            secret_from_env: self.cfg.client_secret_from_env,
            has_token: secrets.bot_token.is_some(),
            has_secret: secrets.client_secret.is_some(),
            repair,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SetupForm {
    step: String,
    #[serde(default)]
    csrf: String,
    #[serde(default)]
    value: String,
}

fn setup_actor() -> Actor {
    Actor {
        user: None,
        name: Some("setup".to_owned()),
        via: Via::Web,
    }
}

/// `POST /setup`
pub async fn submit(
    State(st): State<WebState>,
    ConnectInfo(super::tls::Peer(addr)): ConnectInfo<super::tls::Peer>,
    headers: HeaderMap,
    Form(f): Form<SetupForm>,
) -> Response {
    let loc = locale_of(&headers);
    let secure = https(&headers);
    let back = |notice: Option<(bool, String)>, extra: Option<HeaderValue>| {
        let mut cookies: Vec<HeaderValue> = extra.into_iter().collect();
        if let Some((ok, t)) = notice {
            cookies.push(st.notices.put(ok, t, secure));
        }
        redirect_with("/setup", cookies)
    };
    if st.secrets.get().setup.done && !st.repairing() {
        return redirect_with("/", vec![]);
    }
    if !same_origin(&headers) {
        return back(Some((false, text(loc, "form-expired", &[]))), None);
    }
    if f.step == "code" {
        let ip = addr.ip();
        let mut w = st.wizard();
        if !w.wait(Some(ip)).is_zero() {
            return back(None, None);
        }
        if !w.code.as_deref().is_some_and(|c| code_matches(&f.value, c)) {
            let g = w.guesses.entry(ip).or_insert(Guess {
                fails: 0,
                until: Instant::now(),
            });
            g.fails += 1;
            g.until = Instant::now() + delay(g.fails);
            tracing::warn!(%ip, fails = g.fails, "a wrong setup code was entered");
            return back(Some((false, text(loc, "setup-code-wrong", &[]))), None);
        }
        w.guesses.remove(&ip);
        let id = random_token(32);
        w.sessions.retain(|_, s| s.started.elapsed() < SESSION_FOR);
        w.sessions.insert(
            sha256_hex(&id),
            WizardSession {
                started: Instant::now(),
                // Repairing: the instance is known (and can be opened again).
                instance_ok: st.secrets.get().setup.done,
                revisit: None,
                typed_instance: None,
            },
        );
        let c = set_cookie(
            SETUP_COOKIE,
            &st.sessions.signer().seal("setup", &id),
            Some(SESSION_FOR),
            secure,
        );
        return back(None, Some(c));
    }
    let Some(key) = st.wizard_session(&headers) else {
        return back(Some((false, text(loc, "setup-expired", &[]))), None);
    };
    if !st.sessions.signer().check("setup-csrf", &key, &f.csrf) {
        return back(Some((false, text(loc, "form-expired", &[]))), None);
    }
    let value = f.value.trim().to_owned();
    let revisit = |step: Option<SetupStep>| {
        if let Some(s) = st.wizard().sessions.get_mut(&key) {
            s.revisit = step;
        }
    };
    match f.step.as_str() {
        // Open a done step again; "keep" goes on without changing it.
        "goto" => {
            let target = match value.as_str() {
                "instance" => Some(SetupStep::Instance),
                "token" => Some(SetupStep::Token),
                "secret" => Some(SetupStep::ClientSecret),
                _ => None,
            };
            revisit(target);
            back(None, None)
        }
        "keep" => {
            revisit(None);
            back(None, None)
        }
        "instance" => {
            // What was typed stays in the field when it does not work (not the saved address).
            let typed = |v: Option<String>| {
                if let Some(s) = st.wizard().sessions.get_mut(&key) {
                    s.typed_instance = v;
                }
            };
            let origin: InstanceUrl = match value.parse() {
                Ok(o) => o,
                Err(e) => {
                    typed(Some(value.clone()));
                    return back(Some((false, pb_i18n::value_error(loc, &e))), None);
                }
            };
            if let Err(e) = st.engine.discover(origin.url()).await {
                typed(Some(value.clone()));
                return back(Some((false, pb_web::fmt::engine_error(loc, &e))), None);
            }
            typed(None);
            let set = st
                .engine
                .settings()
                .change(setup_actor(), |t| {
                    Ok(t.set(
                        Scope::Global,
                        SettingKey::Instance,
                        serde_json::json!(origin.to_string()),
                        true,
                    )?
                    .into_iter()
                    .collect())
                })
                .await;
            if let Err(e) = set {
                return back(Some((false, super::forms::change_error(loc, &e))), None);
            }
            if let Some(s) = st.wizard().sessions.get_mut(&key) {
                s.instance_ok = true;
                s.revisit = None;
            }
            back(None, None)
        }
        "token" => match use_token(&st, value, loc).await {
            Ok(()) => {
                revisit(None);
                // Repairing: whether the saved client secret still works decides if that step comes next.
                if st.secrets.get().setup.done
                    && let Some(secret) = st.secrets.get().client_secret
                    && let Ok(ok) = secret_accepted(&st, &secret, loc).await
                {
                    st.client_rejected.store(!ok, Ordering::Relaxed);
                }
                st.refresh_code().await;
                back(None, None)
            }
            Err(e) => back(Some((false, e)), None),
        },
        "secret" => {
            if value.is_empty() {
                return back(None, None);
            }
            if st.cfg.client_secret_from_env {
                return back(Some((false, text(loc, "ui-secret-from-env", &[]))), None);
            }
            if let Err(e) = use_client_secret(&st, SecretString::from(value), loc).await {
                return back(Some((false, e)), None);
            }
            // The redirect address shown is the one logins will use: keep it.
            if st
                .engine
                .settings()
                .current()
                .effective(None, None)
                .ui_url
                .value
                .is_none()
            {
                let origin = request_origin(&headers);
                let set = st
                    .engine
                    .settings()
                    .change(setup_actor(), |t| {
                        Ok(
                            t.set(Scope::Global, SettingKey::UiUrl, serde_json::json!(origin), true)?
                                .into_iter()
                                .collect(),
                        )
                    })
                    .await;
                if let Err(e) = set {
                    return back(Some((false, super::forms::change_error(loc, &e))), None);
                }
            }
            revisit(None);
            back(None, None)
        }
        _ => back(None, None),
    }
}

/// Saves a bot token and logs in with it; the reason (in words) when that fails. A token Fluxer rejects is not kept:
/// the one before it is put back (a typo does not take the bot offline).
pub(crate) async fn use_token(st: &WebState, token: String, loc: Locale) -> Result<(), String> {
    let well_formed = token
        .split_once('.')
        .is_some_and(|(id, secret)| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && !secret.is_empty());
    if !well_formed {
        return Err(text(loc, "setup-token-format", &[]));
    }
    let before = st.secrets.get().bot_token.filter(|b| b.expose_secret() != token);
    st.secrets
        .update(|s| s.bot_token = Some(SecretString::from(token)))
        .await
        .map_err(|e| pb_web::fmt::store_error(loc, &e))?;
    let outcome = log_in(st, loc).await;
    if outcome.is_err()
        && st.engine.connection() == Connection::TokenRejected
        && let Some(before) = before
    {
        st.secrets
            .update(|s| s.bot_token = Some(before))
            .await
            .map_err(|e| pb_web::fmt::store_error(loc, &e))?;
        st.engine.reconnect();
    }
    outcome
}

/// Saves a client secret once Fluxer accepts it; the reason (in words) when it does not, or cannot be asked.
pub(crate) async fn use_client_secret(st: &WebState, secret: SecretString, loc: Locale) -> Result<(), String> {
    if !secret_accepted(st, &secret, loc).await? {
        return Err(text(loc, "setup-secret-rejected", &[]));
    }
    st.secrets
        .update(|s| s.client_secret = Some(secret))
        .await
        .map_err(|e| pb_web::fmt::store_error(loc, &e))?;
    st.client_rejected.store(false, Ordering::Relaxed);
    st.refresh_code().await;
    Ok(())
}

/// Whether Fluxer accepts `secret` for the bot's application (the one the bot token names).
async fn secret_accepted(st: &WebState, secret: &SecretString, loc: Locale) -> Result<bool, String> {
    let Some(client_id) = st.secrets.client_id() else {
        return Err(text(
            loc,
            "login-not-ready",
            &[("reason", text(loc, "login-no-token", &[]).into())],
        ));
    };
    let ep = st
        .engine
        .login_endpoints()
        .await
        .map_err(|e| pb_web::fmt::engine_error(loc, &e))?;
    st.engine
        .client_secret_ok(&ep, client_id, secret)
        .await
        .map_err(|e| text(loc, "login-unreachable", &[("error", e.to_string().into())]))
}

/// Keeps the setup code in step with the bot's credentials: one is made (and logged) as soon as Fluxer rejects the
/// bot token, and dropped once the credentials work again.
pub(crate) async fn watch_credentials(st: WebState) {
    let mut connection = st.engine.watch_connection();
    loop {
        st.refresh_code().await;
        if connection.changed().await.is_none() {
            return;
        }
    }
}

/// Connects to Fluxer again and waits for the outcome; the reason (in words) when it fails.
pub(crate) async fn log_in(st: &WebState, loc: Locale) -> Result<(), String> {
    let attempt = st.engine.reconnect();
    match st.engine.login_outcome(attempt, LOGIN_WAIT).await {
        Connection::Ready => Ok(()),
        Connection::TokenRejected => {
            let instance = st
                .engine
                .settings()
                .current()
                .effective(None, None)
                .instance
                .value
                .to_string();
            Err(text(loc, "setup-token-rejected-at", &[("instance", instance.into())]))
        }
        Connection::NoVoice => Err(text(loc, "ui-fluxer-no-voice", &[])),
        Connection::Retrying(e) | Connection::Stopped(e) => {
            Err(text(loc, "setup-token-unreachable", &[("error", e.into())]))
        }
        Connection::Connecting | Connection::NoToken => Err(text(loc, "setup-token-timeout", &[])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guessing_gets_slower() {
        assert_eq!(delay(1), Duration::from_secs(1));
        assert_eq!(delay(2), Duration::from_secs(2));
        assert_eq!(delay(9), Duration::from_secs(256));
        assert_eq!(delay(50), Duration::from_secs(256));
    }
}
