//! Lists of communities, roles and people in the settings (the allowed communities, who is tracked in every community,
//! the extra bot owners, admin roles). A list is never edited as a whole: each entry is shown by name with its own
//! "Remove" (confirmed first), and one entry at a time is added by name or id. The server changes one entry inside a
//! settings change, so two people editing at once never drop each other's entries.

use std::collections::BTreeMap;

use leptos::prelude::*;
use pb_domain::{GuildId, Scope};
use pb_i18n::text;
use pb_live_proto::Who;
use pb_settings::SettingKey;
use serde_json::Value;

use super::settings::scope_param;
use crate::app::{Viewer, app};
use crate::islands::{Avatar, MemberPicker};

type Engine = pb_engine::Engine;

/// What a list holds (`FieldKind::Ids { of }`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Of {
    Community,
    Role,
    User,
}

impl Of {
    pub fn for_key(key: SettingKey) -> Option<Of> {
        match key.meta().kind {
            pb_settings::FieldKind::Ids { of: "community" } => Some(Of::Community),
            pb_settings::FieldKind::Ids { of: "role" } => Some(Of::Role),
            pb_settings::FieldKind::Ids { .. } => Some(Of::User),
            _ => None,
        }
    }
}

/// The ids in a list's value (JSON: ids as strings or numbers).
pub fn ids_of(value: &Value) -> Vec<u64> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
                .collect()
        })
        .unwrap_or_default()
}

/// The communities whose roles a list at `scope` can hold: the scope's own, or every community.
fn role_guilds(engine: &Engine, scope: Scope) -> Vec<GuildId> {
    let gs = engine.guilds();
    gs.guilds
        .keys()
        .copied()
        .filter(|g| scope.guild().is_none_or(|s| s == *g))
        .collect()
}

/// A person as the bot knows them from any community (name and picture), else by id.
pub fn person(engine: &Engine, user: u64) -> Who {
    let u = pb_domain::UserId(user);
    let gs = engine.guilds();
    let seen = gs
        .guilds
        .iter()
        .find(|(_, i)| i.people.contains_key(&u))
        .map(|(g, _)| *g)
        .or_else(|| gs.known.keys().find(|(_, x)| *x == u).map(|(g, _)| *g));
    match seen {
        Some(g) => engine.who(g, u),
        None => Who {
            user: u,
            name: user.to_string(),
            avatar: None,
        },
    }
}

/// An entry's name and where it is from (a role's community), and whether the bot knows it at all.
pub fn entry(engine: &Engine, of: Of, scope: Scope, id: u64) -> (String, Option<String>, bool) {
    let gs = engine.guilds();
    match of {
        Of::Community => {
            let g = GuildId(id);
            let known = gs.guilds.contains_key(&g) || gs.known_communities.contains_key(&g);
            (gs.guild_name(g), None, known)
        }
        Of::Role => {
            let role = pb_domain::RoleId(id);
            role_guilds(engine, scope)
                .into_iter()
                .find_map(|g| {
                    let info = gs.get(g)?;
                    let r = info.roles.get(&role)?;
                    Some((r.name.clone(), scope.guild().is_none().then(|| info.name.clone()), true))
                })
                .unwrap_or_else(|| (id.to_string(), None, false))
        }
        Of::User => {
            let who = person(engine, id);
            let known = who.name != id.to_string();
            (who.name, None, known)
        }
    }
}

/// What a typed entry names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// One entry (and whether the bot knows it).
    One {
        id: u64,
        known: bool,
    },
    Nothing,
    /// Several match: their names.
    Several(Vec<String>),
}

/// Everything known by name that a list of `of` at `scope` can hold: id → name.
pub fn known(engine: &Engine, of: Of, scope: Scope, may_see: &dyn Fn(GuildId) -> bool) -> BTreeMap<u64, String> {
    let gs = engine.guilds();
    match of {
        Of::Community => gs
            .guilds
            .keys()
            .chain(gs.known_communities.keys())
            .filter(|g| may_see(**g))
            .map(|g| (g.0, gs.guild_name(*g)))
            .collect(),
        Of::Role => role_guilds(engine, scope)
            .into_iter()
            .filter(|g| may_see(*g))
            .filter_map(|g| gs.get(g).map(|i| (g, i)))
            .flat_map(|(g, i)| {
                i.roles
                    .values()
                    .filter(move |r| r.id.0 != g.0)
                    .map(|r| (r.id.0, r.name.clone()))
                    .collect::<Vec<_>>()
            })
            .collect(),
        Of::User => {
            let mut out = BTreeMap::new();
            for (g, info) in &gs.guilds {
                if may_see(*g) {
                    for p in info.people.values().filter(|p| !p.bot) {
                        out.entry(p.user.0).or_insert_with(|| p.shown());
                    }
                }
            }
            for ((g, u), p) in &gs.known {
                if may_see(*g) && !p.bot {
                    out.entry(u.0).or_insert_with(|| p.shown());
                }
            }
            out
        }
    }
}

/// Finds what was typed: an id or a mention as it is (known or not), else a name among what the bot knows (the same
/// name in any case; else names that start with it), and for people the communities' member search.
pub async fn resolve(
    engine: &Engine,
    of: Of,
    scope: Scope,
    typed: &str,
    may_see: &(dyn Fn(GuildId) -> bool + Sync),
) -> Found {
    let typed = typed.trim();
    let id = pb_domain::unmention(typed).parse::<u64>().ok();
    if let Some(id) = id {
        let (_, _, known) = entry(engine, of, scope, id);
        return Found::One { id, known };
    }
    let names = known(engine, of, scope, may_see);
    let lower = typed.to_lowercase();
    let pick = |hits: Vec<(&u64, &String)>| match hits.as_slice() {
        [] => None,
        [(id, _)] => Some(Found::One { id: **id, known: true }),
        many => Some(Found::Several(many.iter().map(|(_, n)| (*n).clone()).collect())),
    };
    let exact: Vec<_> = names.iter().filter(|(_, n)| n.to_lowercase() == lower).collect();
    if let Some(f) = pick(exact) {
        return f;
    }
    let start: Vec<_> = names
        .iter()
        .filter(|(_, n)| n.to_lowercase().starts_with(&lower))
        .collect();
    if let Some(f) = pick(start) {
        return f;
    }
    if of == Of::User {
        // People the bot has not met yet: Fluxer's member search in each community this login sees.
        let mut found: BTreeMap<u64, String> = BTreeMap::new();
        for g in engine.guilds().available().into_iter().filter(|g| may_see(*g)) {
            if let Ok(people) = engine.search_members(g, typed, 10).await {
                for w in people {
                    found.entry(w.user.0).or_insert(w.name);
                }
            }
        }
        let hits: Vec<_> = found.iter().collect();
        if let Some(f) = pick(hits) {
            return f;
        }
    }
    Found::Nothing
}

/// A list setting's entries and its "Add" field (the row's head and badges come from the settings form). `value`: the
/// list in effect at `scope`; `disabled`: this login may only look.
#[component]
pub fn ListBody(
    key: SettingKey,
    scope: Scope,
    value: Value,
    viewer: Viewer,
    back: String,
    disabled: bool,
    is_here: bool,
) -> impl IntoView {
    let loc = viewer.locale;
    let Some(of) = Of::for_key(key) else {
        return ().into_any();
    };
    let engine = app().engine;
    let ids = ids_of(&value);
    let fields = |op: &str| {
        vec![
            ("csrf".to_owned(), viewer.csrf.clone()),
            ("scope".to_owned(), scope_param(scope)),
            ("key".to_owned(), key.name()),
            ("back".to_owned(), back.clone()),
            ("op".to_owned(), op.to_owned()),
        ]
    };
    let hidden = |op: &str| {
        fields(op)
            .into_iter()
            .map(|(k, v)| view! { <input type="hidden" name=k value=v/> })
            .collect_view()
    };
    let items = ids
        .iter()
        .map(|id| {
            let (name, from, _) = entry(&engine, of, scope, *id);
            let named = name != id.to_string();
            let picture = (of == Of::User).then(|| view! { <Avatar who=person(&engine, *id) size=22/> });
            view! {
                <li>
                    {picture}
                    <span class="name">{name}</span>
                    {from.map(|f| view! { <span class="muted small">{f}</span> })}
                    // The id beside a name (an id the bot does not know is shown as the name already).
                    {named.then(|| view! { <span class="muted small mono">{id.to_string()}</span> })}
                    {(!disabled).then(|| view! {
                        <form method="post" action="/settings/list" class="inline">
                            {hidden("remove")}
                            <input type="hidden" name="entry" value=id.to_string()/>
                            <button class="link danger">{text(loc, "ui-remove", &[])}</button>
                        </form>
                    })}
                </li>
            }
        })
        .collect_view();
    let input_id = super::settings::input_id(key);
    let adder = (!disabled).then(|| match of {
        Of::User => view! {
            <MemberPicker guild=scope.guild() action="/settings/list".to_owned() field="entry".to_owned() hidden=fields("add")
                button=text(loc, "ui-list-add", &[]) placeholder=text(loc, "ui-user-id-or-mention", &[]) id=input_id.clone()/>
        }
        .into_any(),
        Of::Community | Of::Role => {
            // The names the browser suggests while typing (no scripts needed); ids work as well.
            let list_id = format!("known-{}", key.name());
            let names: Vec<String> = known(&engine, of, scope, &|g| viewer.may_see(g))
                .into_iter()
                .filter(|(id, _)| !ids.contains(id))
                .map(|(_, n)| n)
                .collect();
            let placeholder = text(
                loc,
                if of == Of::Community { "ui-list-add-community" } else { "ui-list-add-role" },
                &[],
            );
            view! {
                <form method="post" action="/settings/list" class="row">
                    {hidden("add")}
                    <input id=input_id.clone() name="entry" list=list_id.clone() required autocomplete="off" placeholder=placeholder/>
                    <datalist id=list_id>{names.into_iter().map(|n| view! { <option value=n></option> }).collect_view()}</datalist>
                    <button class="button">{text(loc, "ui-list-add", &[])}</button>
                </form>
            }
            .into_any()
        }
    });
    // In a community or for a person the list can go back to the one above; globally entries go one by one.
    let inherit = (is_here && !disabled && scope != Scope::Global).then(|| {
        view! {
            <form method="post" action="/settings" class="inline">
                <input type="hidden" name="csrf" value=viewer.csrf.clone()/>
                <input type="hidden" name="scope" value=scope_param(scope)/>
                <input type="hidden" name="key" value=key.name()/>
                <input type="hidden" name="back" value=back.clone()/>
                <button class="link" name="action" value="clear">{text(loc, "ui-use-inherited", &[])}</button>
            </form>
        }
    });
    view! {
        <ul class="id-list">
            {items}
            {ids.is_empty().then(|| view! { <li class="muted">{text(loc, "ui-list-empty", &[])}</li> })}
        </ul>
        {adder}
        {inherit}
    }
    .into_any()
}
