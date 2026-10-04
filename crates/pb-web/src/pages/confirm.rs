//! `/confirm`: "Really …?" before something that cannot be undone: deleting a recording, removing a clip a voice line
//! uses, no longer tracking someone, emptying a swear jar, resetting every setting at a scope. A form that does one of
//! these and arrives without `confirm=1` sends the browser here (with what it is about, not its token); this page says
//! what will happen and sends the same form again, confirmed. No scripts needed.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use pb_domain::{BlobHash, GuildId, Scope, SentenceId, UserId};
use pb_i18n::{Locale, setting_name, text};

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
