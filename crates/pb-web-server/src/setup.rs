//! The setup wizard's steps (the page is `pb_web::pages::setup`). The setup code proves access to the server; it
//! opens a wizard session (1 h) in which the instance, the bot token and the client secret are set, and ends with an
//! owner login.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use axum::Form;
use axum::extract::{ConnectInfo, State};
use axum::response::Response;
use http::{HeaderMap, HeaderValue};
use pb_domain::Scope;
use pb_engine::Connection;
use pb_i18n::{Locale, text};
use pb_settings::{Origin, SettingKey};
use pb_store_api::{Actor, Via};
use pb_web::app::{SetupStep, SetupView};
use secrecy::SecretString;
use serde::Deserialize;

use super::auth::{SETUP_COOKIE, code_matches, cookie, https, random_token, set_cookie, sha256_hex};
use super::login::{redirect_uri, request_origin};
use super::server::{WebState, redirect_with};
use super::util::{locale_of, same_origin};

const SESSION_FOR: Duration = Duration::from_secs(3600);
/// How long the token step waits for the bot to log in with a new token.
const LOGIN_WAIT: Duration = Duration::from_secs(20);

struct WizardSession {
    started: Instant,
    instance_ok: bool,
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

    pub(crate) fn setup_view(&self, headers: &HeaderMap, ip: Option<IpAddr>) -> SetupView {
        let secrets = self.secrets.get();
        let key = self.wizard_session(headers);
        let (instance_ok, wait) = {
            let w = self.wizard();
            (
                key.as_ref()
                    .and_then(|k| w.sessions.get(k))
                    .is_some_and(|s| s.instance_ok),
                w.wait(ip),
            )
        };
        let step = if secrets.setup.done {
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
        } else if secrets.client_secret.is_none() {
            SetupStep::ClientSecret
        } else {
            SetupStep::Owner
        };
        let eff = self.engine.settings().current().effective(None, None);
        SetupView {
            step,
            csrf: key
                .map(|k| self.sessions.signer().tag("setup-csrf", &k))
                .unwrap_or_default(),
            instance: eff.instance.value.to_string(),
            bot: self
                .engine
                .identity()
                .map(|i| i.user.global_name.clone().unwrap_or(i.user.username.clone())),
            redirect_uri: redirect_uri(self, headers),
            wait_secs: wait.as_secs() + u64::from(wait.subsec_nanos() > 0),
            code_file: self.cfg.setup_code_file.clone(),
            secret_from_env: self.cfg.client_secret_from_env,
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
    if st.secrets.get().setup.done {
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
                instance_ok: false,
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
    match f.step.as_str() {
        "instance" => {
            let origin: Origin = match value.parse() {
                Ok(o) => o,
                Err(e) => return back(Some((false, pb_i18n::value_error(loc, &e))), None),
            };
            if let Err(e) = st.engine.discover(origin.url()).await {
                return back(
                    Some((false, text(loc, "setup-instance-unreachable", &[("error", e.into())]))),
                    None,
                );
            }
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
            }
            back(None, None)
        }
        "token" => match use_token(&st, value, loc).await {
            Ok(()) => back(None, None),
            Err(e) => back(Some((false, e)), None),
        },
        "secret" => {
            if value.is_empty() {
                return back(None, None);
            }
            if st.cfg.client_secret_from_env {
                return back(Some((false, text(loc, "ui-secret-from-env", &[]))), None);
            }
            if let Err(e) = st
                .secrets
                .update(|s| s.client_secret = Some(SecretString::from(value)))
                .await
            {
                return back(Some((false, pb_web::fmt::store_error(loc, &e))), None);
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
            back(None, None)
        }
        _ => back(None, None),
    }
}

/// Saves a bot token and logs in with it; the reason (in words) when that fails.
pub(crate) async fn use_token(st: &WebState, token: String, loc: Locale) -> Result<(), String> {
    let well_formed = token
        .split_once('.')
        .is_some_and(|(id, secret)| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) && !secret.is_empty());
    if !well_formed {
        return Err(text(loc, "setup-token-format", &[]));
    }
    st.secrets
        .update(|s| s.bot_token = Some(SecretString::from(token)))
        .await
        .map_err(|e| pb_web::fmt::store_error(loc, &e))?;
    log_in(st, loc).await
}

/// Connects to Fluxer again and waits for the outcome; the reason (in words) when it fails.
pub(crate) async fn log_in(st: &WebState, loc: Locale) -> Result<(), String> {
    let attempt = st.engine.reconnect();
    match st.engine.login_outcome(attempt, LOGIN_WAIT).await {
        Connection::Ready => Ok(()),
        Connection::TokenRejected => Err(text(loc, "setup-token-rejected", &[])),
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
