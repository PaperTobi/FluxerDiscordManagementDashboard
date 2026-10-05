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

    /// The owner, for replacing the bot token or client secret: with a recent login, unless Fluxer rejects the saved
    /// ones (then nobody can log in again, so the login the owner has must do).
    pub(crate) fn owner_for_credentials(&self, headers: &HeaderMap, f: &Fields) -> Result<Sender, Box<Response>> {
        if self.credentials_broken() {
            self.owner(headers, f)
        } else {
            self.fresh_owner(headers, f)
        }
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

    /// Back to the form's page with a notice and what happened to each field (shown next to it).
    pub(crate) fn done_fields(
        &self,
        s: &Sender,
        f: &Fields,
        ok: bool,
        msg: String,
        notes: Vec<pb_web::app::FieldNote>,
    ) -> Response {
        redirect_with(
            &safe_next(field(f, "back")),
            vec![self.notices.put_fields(ok, msg, notes, s.secure)],
        )
    }
}

/// Whether a form that destroys something was confirmed on the `/confirm` page.
pub(crate) fn confirmed(f: &Fields) -> bool {
    field(f, "confirm") == Some("1")
}

/// Before something that cannot be undone: sends the browser to `/confirm`, which says what `what` will do with the
/// form's `keys` and sends the same form again, confirmed (no scripts needed). The form token stays out of the URL.
pub(crate) fn ask_first(what: &str, f: &Fields, keys: &[&str]) -> Response {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    q.append_pair("what", what);
    for k in keys {
        if let Some(v) = field(f, k) {
            q.append_pair(k, v);
        }
    }
    q.append_pair("back", &safe_next(field(f, "back")));
    redirect_with(&format!("/confirm?{}", q.finish()), vec![])
}

/// A refused settings change, in the reader's language.
pub(crate) fn change_error(loc: Locale, e: &pb_engine::ChangeError) -> String {
    match e {
        pb_engine::ChangeError::Setting(se) => pb_i18n::setting_error(loc, se),
        pb_engine::ChangeError::Store(se) => pb_web::fmt::store_error(loc, se),
    }
}

pub(crate) use pb_web::pages::settings::parse_scope;

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

/// The voice per language or per kind of line (`voice.<lang>` / `line-voice.<kind>` fields: both can be in one
/// section's form; empty = none chosen).
fn voices_json(f: &Fields, key: SettingKey) -> Value {
    let prefix = if key == SettingKey::LineVoices {
        "line-voice."
    } else {
        "voice."
    };
    Value::Object(
        f.iter()
            .filter_map(|(k, v)| Some((k.strip_prefix(prefix)?.to_owned(), v.trim().to_owned())))
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| (k, Value::String(v)))
            .collect(),
    )
}

/// Going back to the inherited value of the System section (the instance, the web UI's address) or of a whole list is
/// asked first: one click could cut the bot off or empty the list.
fn weighty(key: SettingKey) -> bool {
    (pb_web::pages::settings::kept_by_reset(key) && key != SettingKey::Paused)
        || matches!(key.meta().kind, FieldKind::Ids { .. })
}

/// `POST /settings`: a section's form (`keys`: its settings; what changed in it is saved), or one setting (`key`,
/// `action` = `set` or `clear`; the page heads' switches).
pub async fn settings(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let loc = s.locale;
    let Some(scope) = field(&f, "scope").and_then(parse_scope) else {
        return st.done(&s, &f, false, text(loc, "form-expired", &[]));
    };
    if !s.may_change(scope) {
        return st.done(&s, &f, false, text(loc, "ui-not-allowed", &[]));
    }
    if let Some(keys) = field(&f, "keys") {
        return st.save_section(&s, &f, scope, keys).await;
    }
    let Some(key) = field(&f, "key").and_then(|k| k.parse::<SettingKey>().ok()) else {
        return st.done(&s, &f, false, text(loc, "form-expired", &[]));
    };
    let clear = field(&f, "action") == Some("clear");
    if clear && weighty(key) && !confirmed(&f) {
        return ask_first("setting-clear", &f, &["scope", "key"]);
    }
    let value = match key.meta().kind {
        FieldKind::Escalation => escalation_json(&f),
        FieldKind::Voices | FieldKind::LineVoices => voices_json(&f, key),
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
    // The result is shown next to the setting too; a refused value stays in the field as it was typed.
    let note = |ok: bool, msg: &str| pb_web::app::FieldNote {
        key: key.name(),
        ok,
        text: msg.to_owned(),
        typed: (!ok).then(|| field(&f, "value").map(str::to_owned)).flatten(),
    };
    let name = pb_i18n::setting_name(loc, key);
    match result {
        Ok(changes) if changes.is_empty() => st.done(&s, &f, true, text(loc, "ui-unchanged", &[])),
        Err(e) => {
            let msg = change_error(loc, &e);
            st.done_fields(&s, &f, false, msg.clone(), vec![note(false, &msg)])
        }
        // Pausing says what it does, and where.
        Ok(_) if key == SettingKey::Paused => {
            let paused = !clear && field(&f, "value") == Some("on");
            let id = if paused { "ui-paused-done" } else { "ui-resumed-done" };
            st.done(
                &s,
                &f,
                true,
                text(loc, id, &[("where", st.scope_words(loc, scope).into())]),
            )
        }
        Ok(_) => st.done_fields(
            &s,
            &f,
            true,
            text(loc, "ui-saved-what", &[("what", name.into())]),
            vec![note(true, &text(loc, "ui-saved", &[]))],
        ),
    }
}

/// A setting's value as a section's form sends it (and what was typed, to show again when it is refused).
fn form_value(f: &Fields, key: SettingKey) -> (Value, Option<String>) {
    match key.meta().kind {
        FieldKind::Escalation => (escalation_json(f), None),
        FieldKind::Voices | FieldKind::LineVoices => (voices_json(f, key), None),
        _ => {
            let raw = field(f, &pb_web::pages::settings::input_name(key)).unwrap_or_default();
            (pb_settings::text_value(key, raw), Some(raw.to_owned()))
        }
    }
}

impl WebState {
    /// A section's form: every setting whose value it changed is saved in one settings change. "Changed" means the
    /// value in effect at the scope would be different: what the form shows unchanged (an inherited value too) is
    /// never stored, so it keeps following the scope above. A refused value is said next to its setting (and shown
    /// again as typed); the others are saved. `clear=<key>` also takes that one back to its inherited value.
    async fn save_section(&self, s: &Sender, f: &Fields, scope: Scope, keys: &str) -> Response {
        use pb_web::app::FieldNote;
        use pb_web::pages::settings::{advanced, effective_value};
        let loc = s.locale;
        let by_owner = s.access.owner;
        let keys: Vec<SettingKey> = keys
            .split(',')
            .filter_map(|k| k.parse::<SettingKey>().ok())
            .filter(|k| {
                k.scopes().contains(&scope.kind())
                    && (by_owner || k.who() != pb_settings::Who::Owner)
                    && !matches!(k.meta().kind, FieldKind::Ids { .. })
            })
            .collect();
        let clear = field(f, "clear")
            .and_then(|k| k.parse::<SettingKey>().ok())
            .filter(|k| keys.contains(k));
        if let Some(k) = clear
            && weighty(k)
            && !confirmed(f)
        {
            let mut q = url::form_urlencoded::Serializer::new(String::new());
            q.append_pair("what", "setting-clear")
                .append_pair("scope", &pb_web::pages::settings::scope_param(scope))
                .append_pair("key", &k.name())
                .append_pair("back", &safe_next(field(f, "back")));
            return redirect_with(&format!("/confirm?{}", q.finish()), vec![]);
        }
        // What the form changed: each value checked on its own against the settings as they are.
        let now = self.engine.settings().current().tree().clone();
        let mut wanted: Vec<(SettingKey, Value)> = Vec::new();
        let mut notes: Vec<FieldNote> = Vec::new();
        let mut refused: Vec<SettingKey> = Vec::new();
        for key in keys.iter().copied().filter(|k| Some(*k) != clear) {
            let (value, typed) = form_value(f, key);
            let mut probe = now.clone();
            match probe.set(scope, key, value.clone(), by_owner) {
                Err(e) => {
                    notes.push(FieldNote {
                        key: key.name(),
                        ok: false,
                        text: pb_i18n::setting_error(loc, &e),
                        typed,
                    });
                    refused.push(key);
                }
                Ok(_) if effective_value(&probe, scope, key).0 != effective_value(&now, scope, key).0 => {
                    wanted.push((key, value));
                }
                Ok(_) => {}
            }
        }
        let changed: Vec<SettingKey> = wanted.iter().map(|(k, _)| *k).chain(clear).collect();
        let result = if changed.is_empty() {
            Ok(Vec::new())
        } else {
            self.engine
                .settings()
                .change(s.actor(), move |t| {
                    let mut out = Vec::new();
                    for (key, value) in wanted {
                        out.extend(t.set(scope, key, value, by_owner)?);
                    }
                    if let Some(k) = clear {
                        out.extend(t.clear(scope, k, by_owner)?);
                    }
                    Ok(out)
                })
                .await
        };
        if let Err(e) = result {
            return self.done(s, f, false, change_error(loc, &e));
        }
        let names = |keys: &[SettingKey]| {
            keys.iter()
                .map(|k| pb_i18n::setting_name(loc, *k))
                .collect::<Vec<_>>()
                .join(", ")
        };
        for k in &changed {
            let said = if Some(*k) == clear {
                "ui-back-to-inherited"
            } else {
                "ui-saved"
            };
            notes.push(FieldNote {
                key: k.name(),
                ok: true,
                text: text(loc, said, &[]),
                typed: None,
            });
        }
        let mut said = Vec::new();
        if !changed.is_empty() {
            said.push(text(loc, "ui-saved-what", &[("what", names(&changed).into())]));
        }
        if !refused.is_empty() {
            said.push(text(loc, "ui-section-refused", &[("what", names(&refused).into())]));
        }
        if said.is_empty() {
            said.push(text(loc, "ui-unchanged", &[]));
        }
        // Back to the section, with its "Advanced" part open when a setting in there was saved or refused, and at the
        // first refused setting.
        let mut back = safe_next(field(f, "back"));
        if let Some(section) = changed
            .iter()
            .chain(&refused)
            .find(|k| advanced(**k))
            .map(|k| k.meta().section)
        {
            back = format!("{back}?advanced={}", section.key());
        }
        if let Some(k) = refused.first() {
            back = format!("{back}#set-{}", k.name());
        }
        redirect_with(
            &back,
            vec![
                self.notices
                    .put_fields(refused.is_empty(), said.join(" "), notes, s.secure),
            ],
        )
    }

    /// Where a scope is, in words: "in every community", "in Alpha", "for Max in Alpha".
    pub(crate) fn scope_words(&self, loc: Locale, scope: Scope) -> String {
        let gs = self.engine.guilds();
        match scope {
            Scope::Global => text(loc, "ui-in-every-community", &[]),
            Scope::Server { guild } => text(loc, "audit-in", &[("community", gs.guild_name(guild).into())]),
            Scope::Person { guild, user } => text(
                loc,
                "audit-for",
                &[
                    ("person", gs.name(guild, user).into()),
                    ("community", gs.guild_name(guild).into()),
                ],
            ),
        }
    }
}

/// `POST /settings/reset`: removes every setting at one scope (after a confirmation), so all are inherited again;
/// the pause switches and the System section stay. Community admins leave what only the owner may change.
pub async fn reset(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let loc = s.locale;
    let Some(scope) = field(&f, "scope").and_then(parse_scope) else {
        return st.done(&s, &f, false, text(loc, "form-expired", &[]));
    };
    if !s.may_change(scope) {
        return st.done(&s, &f, false, text(loc, "ui-not-allowed", &[]));
    }
    if !confirmed(&f) {
        return ask_first("reset", &f, &["scope"]);
    }
    let by_owner = s.access.owner;
    let keep: Vec<SettingKey> = SettingKey::all()
        .into_iter()
        .filter(|k| pb_web::pages::settings::kept_by_reset(*k))
        .collect();
    let result = st
        .engine
        .settings()
        .change(s.actor(), move |t| Ok(t.reset(scope, &keep, by_owner)))
        .await;
    match result {
        Ok(changes) if changes.is_empty() => st.done(&s, &f, true, text(loc, "ui-unchanged", &[])),
        Ok(changes) => st.done(
            &s,
            &f,
            true,
            text(loc, "ui-settings-reset", &[("count", changes.len().into())]),
        ),
        Err(e) => st.done(&s, &f, false, change_error(loc, &e)),
    }
}

/// `POST /settings/list`: adds one entry to a list of communities, roles or people (`op=add`, `entry` = a name, id
/// or mention), or removes one (`op=remove`, `entry` = its id; after a confirmation). The change is made on the
/// current settings, so entries others added meanwhile stay.
pub async fn list(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    use pb_web::pages::lists::{Found, Of, entry, ids_of, resolve};
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
    let Some(of) = Of::for_key(key) else {
        return st.done(&s, &f, false, text(loc, "form-expired", &[]));
    };
    if !s.may_change(scope) {
        return st.done(&s, &f, false, text(loc, "ui-not-allowed", &[]));
    }
    let typed = field(&f, "entry").unwrap_or_default().trim().to_owned();
    let remove = field(&f, "op") == Some("remove");
    let (id, known) = if remove {
        match typed.parse::<u64>() {
            Ok(id) => (id, true),
            Err(_) => return st.done(&s, &f, false, text(loc, "form-expired", &[])),
        }
    } else {
        if typed.is_empty() {
            return st.done(&s, &f, false, text(loc, "ui-list-add-empty", &[]));
        }
        let may_see = |g: GuildId| s.may_see(g);
        match resolve(&st.engine, of, scope, &typed, &may_see).await {
            Found::One { id, known } => (id, known),
            Found::Nothing => {
                return st.done(&s, &f, false, text(loc, "ui-list-no-match", &[("name", typed.into())]));
            }
            Found::Several(names) => {
                return st.done(
                    &s,
                    &f,
                    false,
                    text(
                        loc,
                        "ui-list-several",
                        &[("name", typed.into()), ("matches", names.join(", ").into())],
                    ),
                );
            }
        }
    };
    if remove && !confirmed(&f) {
        return ask_first("list-remove", &f, &["scope", "key", "entry"]);
    }
    let name = entry(&st.engine, of, scope, id).0;
    let by_owner = s.access.owner;
    // Read, change one entry, write: inside the settings change, so nothing anyone else changed is lost.
    let result = st
        .engine
        .settings()
        .change(s.actor(), move |t| {
            let mut ids = ids_of(&pb_web::pages::settings::effective_value(t, scope, key).0);
            let had = ids.contains(&id);
            if had != remove {
                return Ok(Vec::new());
            }
            if remove {
                ids.retain(|x| *x != id);
            } else {
                ids.push(id);
            }
            let value = Value::Array(ids.into_iter().map(|x| Value::String(x.to_string())).collect());
            Ok(t.set(scope, key, value, by_owner)?.into_iter().collect())
        })
        .await;
    let args = [("name", name.into())];
    match (result, remove) {
        (Err(e), _) => st.done(&s, &f, false, change_error(loc, &e)),
        (Ok(c), false) if c.is_empty() => st.done(&s, &f, false, text(loc, "ui-list-already", &args)),
        (Ok(c), true) if c.is_empty() => st.done(&s, &f, false, text(loc, "ui-list-not-there", &args)),
        (Ok(_), false) if !known => st.done(&s, &f, true, text(loc, "ui-list-added-unknown", &args)),
        (Ok(_), false) => st.done(&s, &f, true, text(loc, "ui-list-added", &args)),
        (Ok(_), true) => st.done(&s, &f, true, text(loc, "ui-list-removed", &args)),
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

/// `POST /people/untrack` (after a confirmation)
pub async fn untrack(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some((g, u)) = guild_user(&s, &f) else {
        return st.done(&s, &f, false, text(s.locale, "ui-not-a-user", &[]));
    };
    if !confirmed(&f) {
        return ask_first("untrack", &f, &["guild", "user"]);
    }
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

/// `POST /jar/reset` (after a confirmation)
pub async fn jar_reset(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.sender(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some((g, u)) = guild_user(&s, &f) else {
        return st.done(&s, &f, false, text(s.locale, "ui-not-allowed", &[]));
    };
    if !confirmed(&f) {
        return ask_first("jar", &f, &["guild", "user"]);
    }
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
    let clip = field(&f, "clip").filter(|c| !c.is_empty());
    let what = match (clip, field(&f, "preset").filter(|p| !p.is_empty())) {
        (Some(c), _) => match c.parse::<pb_domain::BlobHash>() {
            Ok(h) => pb_engine::SayWhat::Clip(h),
            Err(_) => return st.done(&s, &f, false, text(s.locale, "ui-no-such-clip", &[])),
        },
        (None, Some(p)) => pb_engine::SayWhat::Preset(p.to_owned()),
        (None, None) if said.is_empty() => return st.done(&s, &f, false, text(s.locale, "ui-say-empty", &[])),
        (None, None) => pb_engine::SayWhat::Text {
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

/// `POST /evidence/delete` (the owner, freshly logged in, after a confirmation): deletes a sentence's recording.
pub async fn delete_recording(State(st): State<WebState>, headers: HeaderMap, Form(f): Form<Fields>) -> Response {
    let s = match st.fresh_owner(&headers, &f) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(id) = field(&f, "sentence").and_then(|x| x.parse::<pb_domain::SentenceId>().ok()) else {
        return st.done(&s, &f, false, text(s.locale, "form-expired", &[]));
    };
    if !confirmed(&f) {
        return ask_first("recording", &f, &["sentence"]);
    }
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
        let form = f(&[
            ("voice.de", "de_DE-thorsten-high"),
            ("voice.en", ""),
            ("line-voice.warning", "piper:en_US-amy-low"),
            ("csrf", "x"),
        ]);
        assert_eq!(
            voices_json(&form, SettingKey::TtsVoices),
            serde_json::json!({"de": "de_DE-thorsten-high"})
        );
        assert_eq!(
            voices_json(&form, SettingKey::LineVoices),
            serde_json::json!({"warning": "piper:en_US-amy-low"})
        );
    }
}
