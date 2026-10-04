//! Plain HTML forms that change things: each checks the login, the form token and the origin, does one thing, leaves a
//! notice and sends the browser back (they work without scripts).

use axum::Form;
use axum::extract::State;
use axum::response::Response;
use http::HeaderMap;
use pb_domain::{GuildId, Scope, UserId};
use pb_i18n::{Locale, text};
use pb_settings::{FieldKind, SettingKey};
use pb_store_api::{Actor, Via};
use serde_json::Value;

use super::auth::{ActiveSession, UserAccess, https};
use super::login::safe_next;
use super::server::{WebState, redirect_with};
use super::util::{locale_of, same_origin};
use pb_web::fmt::engine_error;

/// The fields of a form, in order (repeated names allowed).
pub(crate) type Fields = Vec<(String, String)>;

pub(crate) fn field<'a>(f: &'a Fields, name: &str) -> Option<&'a str> {
    f.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

pub(crate) fn fields<'a>(f: &'a Fields, name: &str) -> Vec<&'a str> {
    f.iter().filter(|(k, _)| k == name).map(|(_, v)| v.as_str()).collect()
}

/// Who sent a form.
pub(crate) struct Sender {
    pub login: ActiveSession,
    pub access: UserAccess,
    pub locale: Locale,
    pub secure: bool,
}

impl Sender {
    pub fn may_see(&self, g: GuildId) -> bool {
        self.access.may_see(g)
    }

    /// May change things at `scope`: global only the owner, a community or person its admins.
    pub fn may_change(&self, scope: Scope) -> bool {
        match scope {
            Scope::Global => self.access.owner,
            Scope::Server { guild } | Scope::Person { guild, .. } => self.may_see(guild),
        }
    }

    pub fn actor(&self) -> Actor {
        Actor {
            user: Some(self.login.record.user),
            name: Some(self.login.record.name.clone()),
            via: Via::Web,
        }
    }
}

impl WebState {
    /// The owner; otherwise the response to send instead.
    pub(crate) fn owner(&self, headers: &HeaderMap, f: &Fields) -> Result<Sender, Box<Response>> {
        let s = self.sender(headers, f)?;
        if s.access.owner {
            Ok(s)
        } else {
            Err(Box::new(self.done(&s, f, false, text(s.locale, "ui-not-allowed", &[]))))
        }
    }

    /// The owner with a login from the last 15 minutes (for what cannot be undone, or hands out secrets); otherwise
    /// the response (log in again, then come back).
    pub(crate) fn fresh_owner(&self, headers: &HeaderMap, f: &Fields) -> Result<Sender, Box<Response>> {
        let s = self.owner(headers, f)?;
        if s.login.record.fresh_until <= jiff::Timestamp::now() {
            // Back to the page with a link (a redirect from the form straight on to Fluxer would break the
            // page's form-action policy).
            let back = safe_next(field(f, "back"));
            let notice = self
                .notices
                .put_log_in_again(text(s.locale, "login-again", &[]), back.clone(), s.secure);
            return Err(Box::new(redirect_with(&back, vec![notice])));
        }
        Ok(s)
    }

    /// Checks a form's login, token and origin; on failure the response to send instead.
    pub(crate) fn sender(&self, headers: &HeaderMap, f: &Fields) -> Result<Sender, Box<Response>> {
        let locale = locale_of(headers);
        let secure = https(headers);
        let Some(login) = self.sessions.lookup(headers) else {
            let back = safe_next(field(f, "back"));
            let notice = self
                .notices
                .put_log_in_again(text(locale, "login-again", &[]), back.clone(), secure);
            return Err(Box::new(redirect_with(&back, vec![notice])));
        };
        let token_ok = field(f, "csrf").is_some_and(|t| self.sessions.csrf_ok(&login.key, t));
        if !token_ok || !same_origin(headers) {
            let back = safe_next(field(f, "back"));
            let notice = self.notices.put(false, text(locale, "form-expired", &[]), secure);
            return Err(Box::new(redirect_with(&back, vec![notice])));
        }
        let access = self.sessions.access(login.record.user);
        Ok(Sender {
            login,
            access,
            locale,
            secure,
        })
    }

    /// Back to the clip library with a notice, before the form's sender is known (a broken upload).
    pub(crate) fn done_anon(&self, headers: &HeaderMap, ok: bool, msg: String) -> Response {
        redirect_with("/voice-lines", vec![self.notices.put(ok, msg, https(headers))])
    }

    /// Back to the form's page with a notice.
    pub(crate) fn done(&self, s: &Sender, f: &Fields, ok: bool, msg: String) -> Response {
        redirect_with(&safe_next(field(f, "back")), vec![self.notices.put(ok, msg, s.secure)])
    }
}

/// A refused settings change, in the reader's language.
pub(crate) fn change_error(loc: Locale, e: &pb_engine::ChangeError) -> String {
    match e {
        pb_engine::ChangeError::Setting(se) => pb_i18n::setting_error(loc, se),
        pb_engine::ChangeError::Store(se) => pb_web::fmt::store_error(loc, se),
    }
}

/// `global`, `server:<g>`, `person:<g>:<u>`.
pub(crate) fn parse_scope(s: &str) -> Option<Scope> {
    let mut p = s.split(':');
    match (p.next()?, p.next(), p.next()) {
        ("global", None, None) => Some(Scope::Global),
        ("server", Some(g), None) => Some(Scope::Server {
            guild: GuildId(g.parse().ok()?),
        }),
        ("person", Some(g), Some(u)) => Some(Scope::Person {
            guild: GuildId(g.parse().ok()?),
            user: UserId(u.parse().ok()?),
        }),
        _ => None,
    }
}

/// The escalation table's rows as the validator's JSON (rows without a "from" are left out).
fn escalation_json(f: &Fields) -> Value {
    let from = fields(f, "esc.from");
    let action = fields(f, "esc.action");
    let duration = fields(f, "esc.duration");
    let owner = fields(f, "esc.owner");
    let rows: Vec<Value> = (0..from.len())
        .filter(|&i| !from[i].trim().is_empty())
        .map(|i| {
            let mut step = serde_json::json!({
                "from": from[i].trim().parse::<u64>().map_or_else(|_| Value::String(from[i].into()), Value::from),
                "action": action.get(i).copied().unwrap_or("none"),
                "notify_owner": owner.get(i) == Some(&"on"),
            });
            if let Some(d) = duration.get(i).map(|d| d.trim()).filter(|d| !d.is_empty()) {
                step["duration"] = Value::String(d.into());
            }
            step
        })
        .collect();
    Value::Array(rows)
}

/// The voice per language or per kind of line (`voice.<lang>` / `voice.<kind>` fields; empty = none chosen).
fn voices_json(f: &Fields) -> Value {
    Value::Object(
        f.iter()
            .filter_map(|(k, v)| Some((k.strip_prefix("voice.")?.to_owned(), v.trim().to_owned())))
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k, Value::String(v)))
            .collect(),
    )
}

/// `POST /settings`: sets or clears one setting at one scope.
pub async fn settings(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let loc = s.locale;
    let (Some(scope), Some(key)) = (
        field(&f, "scope").and_then(parse_scope),
        field(&f, "key").and_then(|k| k.parse::<SettingKey>().ok()),
    ) else {
        return st.done(&s, &f, false, text(loc, "form-expired", &[]));
    };
    if !s.may_change(scope) {
        return st.done(&s, &f, false, text(loc, "ui-not-allowed", &[]));
    }
    let clear = field(&f, "action") == Some("clear");
    let value = match key.meta().kind {
        FieldKind::Escalation => escalation_json(&f),
        FieldKind::Voices | FieldKind::LineVoices => voices_json(&f),
        _ => pb_settings::text_value(key, field(&f, "value").unwrap_or_default()),
    };
    let by_owner = s.access.owner;
    let result = st
        .engine
        .settings()
        .change(s.actor(), move |t| {
            if clear {
                Ok(t.clear(scope, key, by_owner)?.into_iter().collect())
            } else {
                Ok(t.set(scope, key, value, by_owner)?.into_iter().collect())
            }
        })
        .await;
    match result {
        Ok(changes) if changes.is_empty() => st.done(&s, &f, true, text(loc, "ui-unchanged", &[])),
        Ok(_) => st.done(&s, &f, true, text(loc, "ui-saved", &[])),
        Err(e) => st.done(&s, &f, false, change_error(loc, &e)),
    }
}

// ------------------------------------------------------------------------------------------------ people, jar, say

/// The community and person a form is about, if this login may act on that community.
fn guild_user(s: &Sender, f: &Fields) -> Option<(GuildId, UserId)> {
    let g = field(f, "guild").and_then(|g| g.parse::<u64>().ok()).map(GuildId)?;
    let u = field(f, "user").and_then(UserId::from_mention)?;
    s.may_see(g).then_some((g, u))
}

/// `POST /people/track`
pub async fn track(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some((g, u)) = guild_user(&s, &f) else {
        return st.done(&s, &f, false, text(s.locale, "ui-not-a-user", &[]));
    };
    let name = st.engine.guilds().name(g, u);
    match st.engine.track(g, &[u], s.actor()).await {
        Ok(t) if t.added.is_empty() => st.done(&s, &f, true, text(s.locale, "ui-unchanged", &[])),
        Ok(_) => st.done(
            &s,
            &f,
            true,
            text(s.locale, "ui-now-tracking", &[("name", name.into())]),
        ),
        Err(pb_engine::TrackError::TheBot) => st.done(&s, &f, false, text(s.locale, "ui-track-the-bot", &[])),
        Err(pb_engine::TrackError::Change(e)) => st.done(&s, &f, false, change_error(s.locale, &e)),
    }
}

/// `POST /people/untrack`
pub async fn untrack(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some((g, u)) = guild_user(&s, &f) else {
        return st.done(&s, &f, false, text(s.locale, "ui-not-a-user", &[]));
    };
    let name = st.engine.guilds().name(g, u);
    match st.engine.untrack(g, &[u], s.actor()).await {
        Ok(r) if !r.removed.is_empty() => st.done(
            &s,
            &f,
            true,
            text(s.locale, "ui-no-longer-tracking", &[("name", name.into())]),
        ),
        Ok(r) if !r.everywhere.is_empty() => st.done(
            &s,
            &f,
            false,
            text(s.locale, "ui-tracked-everywhere", &[("name", name.into())]),
        ),
        Ok(_) => st.done(&s, &f, true, text(s.locale, "ui-unchanged", &[])),
        Err(e) => st.done(&s, &f, false, change_error(s.locale, &e)),
    }
}

/// `POST /jar/reset`
pub async fn jar_reset(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some((g, u)) = guild_user(&s, &f) else {
        return st.done(&s, &f, false, text(s.locale, "ui-not-allowed", &[]));
    };
    st.engine.reset_jar(g, u, s.actor()).await;
    st.done(&s, &f, true, text(s.locale, "ui-jar-emptied", &[]))
}

/// `POST /community/resume-joining`: join voice again where repeated removals paused it.
pub async fn resume_joining(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(g) = field(&f, "guild")
        .and_then(|g| g.parse::<GuildId>().ok())
        .filter(|g| s.may_see(*g))
    else {
        return st.done(&s, &f, false, text(s.locale, "ui-not-allowed", &[]));
    };
    st.engine.resume_joining(g);
    st.done(&s, &f, true, text(s.locale, "ui-joins-resumed", &[]))
}

/// `POST /say`: "Say now" on a person's page.
pub async fn say(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some((g, u)) = guild_user(&s, &f) else {
        return st.done(&s, &f, false, text(s.locale, "ui-not-allowed", &[]));
    };
    let said = field(&f, "text").unwrap_or_default().trim().to_owned();
    let what = match field(&f, "preset").filter(|p| !p.is_empty()) {
        Some(p) => pb_engine::SayWhat::Preset(p.to_owned()),
        None if said.is_empty() => return st.done(&s, &f, false, text(s.locale, "ui-say-empty", &[])),
        None => pb_engine::SayWhat::Text {
            text: said,
            lang: match field(&f, "lang").filter(|l| !l.is_empty()) {
                Some(l) => match l.parse() {
                    Ok(l) => Some(l),
                    Err(_) => return st.done(&s, &f, false, text(s.locale, "form-expired", &[])),
                },
                None => None,
            },
        },
    };
    match st.engine.say_to(g, u, what, s.actor()).await {
        Ok(_) => st.done(&s, &f, true, text(s.locale, "ui-said", &[])),
        Err(e) => st.done(&s, &f, false, engine_error(s.locale, &e)),
    }
}

/// `POST /reports/send` (owner): sends the report now.
pub async fn send_report(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.owner(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    // A report always goes out ("no violations" too), so this also tests that the owner gets direct messages.
    match st.engine.send_digest().await {
        Ok(true) => st.done(&s, &f, true, text(s.locale, "ui-digest-sent", &[])),
        Ok(false) => st.done(&s, &f, false, text(s.locale, "ui-digest-not-sent", &[])),
        Err(e) => st.done(&s, &f, false, engine_error(s.locale, &e)),
    }
}

/// `POST /evidence/delete` (the owner, freshly logged in): deletes a sentence's recording.
pub async fn delete_recording(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.fresh_owner(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(id) = field(&f, "sentence").and_then(|x| x.parse::<pb_domain::SentenceId>().ok()) else {
        return st.done(&s, &f, false, text(s.locale, "form-expired", &[]));
    };
    match st.engine.delete_recording(id, s.actor(), None).await {
        Ok(()) => st.done(&s, &f, true, text(s.locale, "ui-recording-deleted", &[])),
        Err(e) => st.done(&s, &f, false, engine_error(s.locale, &e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(pairs: &[(&str, &str)]) -> Fields {
        pairs.iter().map(|(a, b)| ((*a).to_owned(), (*b).to_owned())).collect()
    }

    #[test]
    fn scopes() {
        assert_eq!(parse_scope("global"), Some(Scope::Global));
        assert_eq!(parse_scope("server:5"), Some(Scope::Server { guild: GuildId(5) }));
        assert_eq!(
            parse_scope("person:5:6"),
            Some(Scope::Person {
                guild: GuildId(5),
                user: UserId(6)
            })
        );
        assert_eq!(parse_scope("server:x"), None);
        assert_eq!(parse_scope("person:5"), None);
    }

    #[test]
    fn escalation_rows() {
        let v = escalation_json(&f(&[
            ("esc.from", "1"),
            ("esc.action", "none"),
            ("esc.duration", ""),
            ("esc.owner", "off"),
            ("esc.from", "3"),
            ("esc.action", "timeout"),
            ("esc.duration", "10m"),
            ("esc.owner", "on"),
            ("esc.from", ""),
            ("esc.action", "none"),
            ("esc.duration", ""),
            ("esc.owner", "off"),
        ]));
        assert_eq!(
            v,
            serde_json::json!([
                {"from": 1, "action": "none", "notify_owner": false},
                {"from": 3, "action": "timeout", "notify_owner": true, "duration": "10m"},
            ])
        );
    }

    #[test]
    fn voices() {
        let v = voices_json(&f(&[
            ("voice.de", "de_DE-thorsten-high"),
            ("voice.en", ""),
            ("csrf", "x"),
        ]));
        assert_eq!(v, serde_json::json!({"de": "de_DE-thorsten-high"}));
    }
}
