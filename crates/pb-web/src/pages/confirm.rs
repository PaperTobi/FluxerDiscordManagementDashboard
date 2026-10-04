//! `/confirm`: "Really …?" before something that cannot be undone: deleting a recording, removing a clip a voice line
//! uses, no longer tracking someone, emptying a swear jar, resetting every setting at a scope, removing a voice line's
//! own clips and texts, resetting what keeps the bot connected or a whole list, logging out on all devices. A form that does one of
//! these (or takes an entry off a list in the settings) and arrives without `confirm=1` sends the browser here (with what it is about, not its token); this page says
//! what will happen and sends the same form again, confirmed. No scripts needed.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use pb_domain::{BlobHash, GuildId, Scope, SentenceId, UserId};
use pb_i18n::{Locale, setting_name, text};
use pb_settings::SettingKey;

use super::NotFound;
use super::settings::scope_param;
use crate::app::{Viewer, app, viewer};
use crate::fmt::local_path;

/// What a confirmation shows and sends.
struct Ask {
    title: String,
    /// What happens, paragraph by paragraph.
    body: Vec<String>,
    /// Items the consequences are about (voice lines, settings).
    list: Vec<String>,
    action: &'static str,
    fields: Vec<(&'static str, String)>,
    button: String,
}

#[component]
pub fn ConfirmPage() -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let q = use_query_map();
    let get = move |k: &str| q.with_untracked(|q| q.get(k)).unwrap_or_default();
    let back = local_path(Some(&get("back")));
    let what = get("what");
    let guild_user = || {
        let g = get("guild")
            .parse::<u64>()
            .ok()
            .map(GuildId)
            .filter(|g| v.may_see(*g))?;
        let u = UserId::from_mention(&get("user"))?;
        Some((g, u))
    };
    let ask: Option<Ask> = match what.as_str() {
        "untrack" => guild_user().map(|(g, u)| untrack(&v, g, u)),
        "clip" => get("clip").parse::<BlobHash>().ok().and_then(|h| clip(&v, h)),
        "reset" => super::settings::parse_scope(&get("scope")).and_then(|s| reset(&v, s)),
        "line-clear" => match (
            super::settings::parse_scope(&get("scope")),
            get("line").parse::<pb_voicelines::LineKey>().ok(),
        ) {
            (Some(s), Some(k)) => line_clear(&v, s, k),
            _ => None,
        },
        "setting-clear" => match (
            super::settings::parse_scope(&get("scope")),
            get("key").parse::<SettingKey>().ok(),
        ) {
            (Some(s), Some(k)) => setting_clear(&v, s, k),
            _ => None,
        },
        "logout-all" => Some(logout_all(&v)),
        "list-remove" => match (
            super::settings::parse_scope(&get("scope")),
            get("key").parse::<SettingKey>().ok(),
            get("entry").parse::<u64>().ok(),
        ) {
            (Some(s), Some(k), Some(id)) => list_remove(&v, s, k, id),
            _ => None,
        },
        "jar" => {
            let Some((g, u)) = guild_user() else {
                return view! { <NotFound/> }.into_any();
            };
            return view! {
                <Suspense fallback=|| ()>
                    {Suspend::new(async move {
                        let ask = jar(&v, g, u).await;
                        view! { <Confirm ask v back/> }
                    })}
                </Suspense>
            }
            .into_any();
        }
        "recording" if v.owner => {
            let Ok(id) = get("sentence").parse::<SentenceId>() else {
                return view! { <NotFound/> }.into_any();
            };
            return view! {
                <Suspense fallback=|| ()>
                    {Suspend::new(async move {
                        match recording(&v, id).await {
                            Some(ask) => view! { <Confirm ask v back/> }.into_any(),
                            None => view! {
                                <p class="notice error">{text(v.locale, "err-no-recording", &[])}</p>
                                <a class="button" href=back>{text(v.locale, "ui-back", &[])}</a>
                            }
                            .into_any(),
                        }
                    })}
                </Suspense>
            }
            .into_any();
        }
        _ => None,
    };
    match ask {
        Some(ask) => view! { <Confirm ask v back/> }.into_any(),
        None => view! { <NotFound/> }.into_any(),
    }
}

/// The question, what happens, and the form that does it (or goes back).
#[component]
fn Confirm(ask: Ask, v: Viewer, back: String) -> impl IntoView {
    let loc = v.locale;
    view! {
        <section class="card confirm" role="alertdialog" aria-labelledby="confirm-title">
            <h1 id="confirm-title">{ask.title}</h1>
            {ask.body.into_iter().map(|p| view! { <p>{p}</p> }).collect_view()}
            {(!ask.list.is_empty()).then(|| view! {
                <ul class="confirm-list">{ask.list.into_iter().map(|i| view! { <li>{i}</li> }).collect_view()}</ul>
            })}
            <form method="post" action=ask.action class="row">
                <input type="hidden" name="csrf" value=v.csrf.clone()/>
                <input type="hidden" name="back" value=back.clone()/>
                {ask.fields.into_iter().map(|(k, val)| view! { <input type="hidden" name=k value=val/> }).collect_view()}
                <input type="hidden" name="confirm" value="1"/>
                <button class="button danger">{ask.button}</button>
                <a class="button" href=back>{text(loc, "ui-cancel", &[])}</a>
            </form>
        </section>
    }
}

fn person_fields(g: GuildId, u: UserId) -> Vec<(&'static str, String)> {
    vec![("guild", g.to_string()), ("user", u.to_string())]
}

fn untrack(v: &Viewer, g: GuildId, u: UserId) -> Ask {
    let loc = v.locale;
    let gs = app().engine.guilds();
    let args = [("name", gs.name(g, u).into()), ("community", gs.guild_name(g).into())];
    Ask {
        title: text(loc, "ui-confirm-untrack", &args),
        body: vec![text(loc, "ui-confirm-untrack-what", &args)],
        list: Vec::new(),
        action: "/people/untrack",
        fields: person_fields(g, u),
        button: text(loc, "ui-untrack", &[]),
    }
}

async fn jar(v: &Viewer, g: GuildId, u: UserId) -> Ask {
    let loc = v.locale;
    let gs = app().engine.guilds();
    let count = app()
        .index
        .jar(Some(g))
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|r| r.user == u)
        .map_or(0, |r| r.count);
    let args = [
        ("name", gs.name(g, u).into()),
        ("community", gs.guild_name(g).into()),
        ("count", count.into()),
    ];
    Ask {
        title: text(loc, "ui-confirm-jar", &args),
        body: vec![text(loc, "ui-confirm-jar-what", &args)],
        list: Vec::new(),
        action: "/jar/reset",
        fields: person_fields(g, u),
        button: text(loc, "ui-jar-reset-button", &[]),
    }
}

async fn recording(v: &Viewer, id: SentenceId) -> Option<Ask> {
    let loc = v.locale;
    let row = app().index.sentence(id).await.ok().flatten()?;
    if row.record.audio.is_none() || row.audio_deleted {
        return None;
    }
    let r = row.record;
    let gs = app().engine.guilds();
    let args = [
        ("name", gs.name(r.guild, r.user).into()),
        ("community", gs.guild_name(r.guild).into()),
        ("when", super::sentences::when(r.started).into()),
    ];
    Some(Ask {
        title: text(loc, "ui-confirm-recording", &[]),
        body: vec![text(loc, "ui-confirm-recording-what", &args)],
        list: Vec::new(),
        action: "/evidence/delete",
        fields: vec![("sentence", id.to_string())],
        button: text(loc, "ui-delete-recording", &[]),
    })
}

/// Where a scope is, for people: "globally", "in Alpha", "for Max in Alpha".
fn scope_where(loc: Locale, s: Scope) -> String {
    let gs = app().engine.guilds();
    match s {
        Scope::Global => text(loc, "audit-globally", &[]),
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

fn clip(v: &Viewer, h: BlobHash) -> Option<Ask> {
    let loc = v.locale;
    let record = app().engine.clip(&h)?;
    let tree = app().engine.settings().current();
    // Lines in communities this login does not manage are counted, not named.
    let (seen, elsewhere): (Vec<_>, Vec<_>) = super::voicelines::clip_uses(&tree, &h)
        .into_iter()
        .partition(|(s, _)| s.guild().is_none_or(|g| v.may_see(g)));
    let mut list: Vec<String> = seen
        .into_iter()
        .map(|(s, k)| format!("{} ({})", super::voicelines::line_title(loc, &k.0), scope_where(loc, s)))
        .collect();
    if !elsewhere.is_empty() {
        list.push(text(
            loc,
            "ui-confirm-clip-elsewhere",
            &[("count", elsewhere.len().into())],
        ));
    }
    let args = [("name", record.name.into())];
    Some(Ask {
        title: text(loc, "ui-confirm-clip", &args),
        body: (!list.is_empty())
            .then(|| text(loc, "ui-confirm-clip-what", &[]))
            .into_iter()
            .collect(),
        list,
        action: "/clips/remove",
        fields: vec![("clip", h.to_string())],
        button: text(loc, "ui-confirm-clip-button", &[]),
    })
}

fn reset(v: &Viewer, scope: Scope) -> Option<Ask> {
    let loc = v.locale;
    let allowed = match scope {
        Scope::Global => v.owner,
        Scope::Server { guild } | Scope::Person { guild, .. } => v.may_see(guild),
    };
    if !allowed {
        return None;
    }
    let list: Vec<String> = super::settings::resettable(scope, v.owner)
        .into_iter()
        .map(|k| setting_name(loc, k))
        .collect();
    let from = text(
        loc,
        match scope {
            Scope::Global => "ui-confirm-reset-from-defaults",
            Scope::Server { .. } => "ui-confirm-reset-from-global",
            Scope::Person { .. } => "ui-confirm-reset-from-community",
        },
        &[],
    );
    let body = if list.is_empty() {
        vec![text(loc, "ui-confirm-reset-nothing", &[])]
    } else {
        vec![
            text(loc, "ui-confirm-reset-kept", &[]),
            text(
                loc,
                "ui-confirm-reset-what",
                &[("where", scope_where(loc, scope).into()), ("from", from.into())],
            ),
        ]
    };
    Some(Ask {
        title: text(loc, "ui-confirm-reset", &[]),
        body,
        list,
        action: "/settings/reset",
        fields: vec![("scope", scope_param(scope))],
        button: text(loc, "ui-reset-settings", &[]),
    })
}

fn list_remove(v: &Viewer, scope: Scope, key: SettingKey, id: u64) -> Option<Ask> {
    use super::lists::{Of, entry, ids_of};
    let loc = v.locale;
    let allowed = match scope {
        Scope::Global => v.owner,
        Scope::Server { guild } | Scope::Person { guild, .. } => v.may_see(guild),
    };
    let of = Of::for_key(key)?;
    if !allowed {
        return None;
    }
    let engine = app().engine;
    let name = entry(&engine, of, scope, id).0;
    let tree = engine.settings().current();
    let ids = ids_of(&super::settings::effective_value(&tree, scope, key).0);
    let args = [("name", name.clone().into())];
    let what = match key {
        SettingKey::GuildAllowlist => "ui-list-remove-guild-allowlist",
        SettingKey::TrackedEverywhere => "ui-list-remove-tracked-everywhere",
        SettingKey::AdminUserIds => "ui-list-remove-admin-user-ids",
        SettingKey::AdminRoleIds => "ui-list-remove-admin-role-ids",
        _ => "ui-list-remove-what",
    };
    let mut body = vec![text(loc, what, &args)];
    if key == SettingKey::GuildAllowlist && ids == [id] {
        body.push(text(loc, "ui-list-remove-last-community", &[]));
    }
    Some(Ask {
        title: text(
            loc,
            "ui-confirm-list-remove",
            &[("name", name.into()), ("setting", setting_name(loc, key).into())],
        ),
        body,
        list: Vec::new(),
        action: "/settings/list",
        fields: vec![
            ("scope", scope_param(scope)),
            ("key", key.name()),
            ("entry", id.to_string()),
            ("op", "remove".to_owned()),
        ],
        button: text(loc, "ui-remove", &[]),
    })
}

/// May this login change things at `scope`?
fn may_change(v: &Viewer, scope: Scope) -> bool {
    match scope {
        Scope::Global => v.owner,
        Scope::Server { guild } | Scope::Person { guild, .. } => v.may_see(guild),
    }
}

fn line_clear(v: &Viewer, scope: Scope, key: pb_voicelines::LineKey) -> Option<Ask> {
    let loc = v.locale;
    if !may_change(v, scope) {
        return None;
    }
    let tree = app().engine.settings().current();
    let slot = tree.voice_lines(scope)?.get(&key)?.clone();
    let line = super::voicelines::line_title(loc, &key.0);
    Some(Ask {
        title: text(loc, "ui-confirm-line-clear", &[("line", line.into())]),
        body: vec![text(
            loc,
            "ui-confirm-line-clear-what",
            &[
                ("where", scope_where(loc, scope).into()),
                ("clips", slot.clips.len().into()),
                ("texts", slot.text.len().into()),
            ],
        )],
        list: Vec::new(),
        action: "/voice-lines",
        fields: vec![
            ("scope", scope_param(scope)),
            ("line", key.to_string()),
            ("op", "clear".to_owned()),
        ],
        button: text(loc, "ui-use-inherited", &[]),
    })
}

fn setting_clear(v: &Viewer, scope: Scope, key: SettingKey) -> Option<Ask> {
    let loc = v.locale;
    if !may_change(v, scope) || (key.who() == pb_settings::Who::Owner && !v.owner) {
        return None;
    }
    // The value it goes back to: the settings as they would be without it.
    let mut without = app().engine.settings().current().tree().clone();
    without.clear(scope, key, true).ok()?;
    let (value, source) = super::settings::effective_value(&without, scope, key);
    let mut body = vec![text(
        loc,
        "ui-confirm-setting-clear-what",
        &[
            (
                "value",
                super::settings::value_label(key, scope.guild(), &value, loc).into(),
            ),
            ("from", text(loc, super::settings::source_id(source), &[]).into()),
        ],
    )];
    if key.apply() == pb_settings::Apply::Reconnect {
        body.push(text(loc, "ui-confirm-setting-clear-reconnect", &[]));
    }
    Some(Ask {
        title: text(
            loc,
            "ui-confirm-setting-clear",
            &[("setting", setting_name(loc, key).into())],
        ),
        body,
        list: Vec::new(),
        action: "/settings",
        fields: vec![
            ("scope", scope_param(scope)),
            ("key", key.name()),
            ("action", "clear".to_owned()),
        ],
        button: text(loc, "ui-use-inherited", &[]),
    })
}

fn logout_all(v: &Viewer) -> Ask {
    let loc = v.locale;
    Ask {
        title: text(loc, "ui-confirm-logout-all", &[]),
        body: vec![text(loc, "ui-confirm-logout-all-what", &[])],
        list: Vec::new(),
        action: "/auth/logout",
        fields: vec![("everywhere", "1".to_owned())],
        button: text(loc, "ui-logout-everywhere", &[]),
    }
}
