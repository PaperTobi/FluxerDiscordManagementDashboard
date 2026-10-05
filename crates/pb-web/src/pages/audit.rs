//! `/audit`: who changed what, logins, actions, messages, the bot starting and stopping (newest first, paged), by
//! kind and community.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use pb_domain::{GuildId, Scope, UserId};
use pb_i18n::{Locale, text};
use pb_settings::{Change, SettingKey};
use pb_store_api::{Actor, AuditFilter, Event, MessagePurpose, Via};

use super::sentences::{cursor_from_query, visible, when};
use super::voicelines::line_title;
use crate::app::{app, viewer};
use crate::fmt;

/// The filter's groups of event kinds.
const GROUPS: &[(&str, &[&str])] = &[
    ("settings", &["settings.changed", "audit.imported"]),
    ("actions", &["action"]),
    ("jar", &["jar.reset", "jar.baseline"]),
    (
        "library",
        &[
            "clip.saved",
            "clip.removed",
            "voice.saved",
            "voice.removed",
            "blob.deleted",
        ],
    ),
    ("logins", &["login"]),
    ("messages", &["message.sent"]),
    ("bot", &["bot.started", "bot.stopped", "log.repaired", "import.done"]),
];

fn person(g: GuildId, u: UserId) -> String {
    app().engine.guilds().name(g, u)
}

fn community(g: GuildId) -> String {
    app().engine.guilds().guild_name(g)
}

fn actor(loc: Locale, a: &Actor) -> String {
    match (&a.name, a.user, a.via) {
        (Some(n), _, _) => n.clone(),
        (None, Some(u), _) => format!("<@{u}>"),
        (None, None, Via::File) => text(loc, "audit-by-file", &[]),
        (None, None, Via::Import) => text(loc, "audit-by-old-bot", &[]),
        (None, None, _) => text(loc, "audit-by-bot", &[]),
    }
}

fn place(loc: Locale, scope: Scope) -> String {
    match scope {
        Scope::Global => text(loc, "audit-globally", &[]),
        Scope::Server { guild } => text(loc, "audit-in", &[("community", community(guild).into())]),
        Scope::Person { guild, user } => text(
            loc,
            "audit-for",
            &[
                ("person", person(guild, user).into()),
                ("community", community(guild).into()),
            ],
        ),
    }
}

fn value(loc: Locale, key: &str, v: &serde_json::Value) -> String {
    key.parse::<SettingKey>()
        .map_or_else(|_| pb_i18n::value_text(loc, v), |k| pb_i18n::setting_value(loc, k, v))
}

fn setting(loc: Locale, key: &str) -> String {
    key.parse::<SettingKey>()
        .map_or_else(|_| key.to_owned(), |k| pb_i18n::setting_name(loc, k))
}

fn change(loc: Locale, by: &str, c: &Change) -> String {
    match c {
        Change::Set { scope, key, after, .. } => text(
            loc,
            "audit-set",
            &[
                ("by", by.into()),
                ("setting", setting(loc, key).into()),
                ("where", place(loc, *scope).into()),
                ("value", value(loc, key, after).into()),
            ],
        ),
        Change::Clear { scope, key, .. } => text(
            loc,
            "audit-clear",
            &[
                ("by", by.into()),
                ("setting", setting(loc, key).into()),
                ("where", place(loc, *scope).into()),
            ],
        ),
        Change::Track { guild, user, .. } | Change::Untrack { guild, user } => text(
            loc,
            if matches!(c, Change::Track { .. }) {
                "audit-track"
            } else {
                "audit-untrack"
            },
            &[
                ("by", by.into()),
                ("person", person(*guild, *user).into()),
                ("community", community(*guild).into()),
            ],
        ),
        Change::VoiceLine { scope, line, .. } => text(
            loc,
            "audit-voice-line",
            &[
                ("by", by.into()),
                ("line", line_title(loc, &line.0).into()),
                ("where", place(loc, *scope).into()),
            ],
        ),
    }
}

/// What an entry says, in a line.
fn summary(loc: Locale, e: &Event) -> String {
    match e {
        Event::SettingsChanged(c) => change(loc, &actor(loc, &c.by), &c.change),
        Event::ImportedAudit(a) => text(
            loc,
            "audit-imported",
            &[
                ("by", actor(loc, &a.actor).into()),
                ("setting", setting(loc, &a.key).into()),
                (
                    "value",
                    a.after
                        .as_ref()
                        .map_or_else(String::new, |v| pb_i18n::value_text(loc, v))
                        .into(),
                ),
            ],
        ),
        Event::Action(a) => text(
            loc,
            "audit-action",
            &[
                ("action", pb_i18n::action_text(loc, a.kind, a.secs).into()),
                ("person", person(a.guild, a.user).into()),
                ("community", community(a.guild).into()),
                ("result", pb_i18n::action_outcome(loc, &a.outcome).into()),
            ],
        ),
        Event::JarReset(j) => text(
            loc,
            "audit-jar-reset",
            &[
                ("by", actor(loc, &j.by).into()),
                ("person", person(j.guild, j.user).into()),
                ("community", community(j.guild).into()),
            ],
        ),
        Event::JarBaseline(j) => text(
            loc,
            "audit-jar-baseline",
            &[
                ("person", person(j.guild, j.user).into()),
                ("community", community(j.guild).into()),
                ("count", j.count.into()),
            ],
        ),
        Event::BlobDeleted(b) => text(loc, "audit-recording-deleted", &[("by", actor(loc, &b.by).into())]),
        Event::ClipSaved(c) => text(
            loc,
            "audit-clip-saved",
            &[("by", actor(loc, &c.by).into()), ("name", c.name.clone().into())],
        ),
        Event::ClipRemoved(c) => text(loc, "audit-clip-removed", &[("by", actor(loc, &c.by).into())]),
        Event::VoiceSaved(v) => text(
            loc,
            "audit-voice-saved",
            &[("by", actor(loc, &v.by).into()), ("name", v.name.clone().into())],
        ),
        Event::VoiceRemoved(v) => text(
            loc,
            "audit-voice-removed",
            &[
                ("by", actor(loc, &v.by).into()),
                ("voice", format!("{}:{}", v.model, v.id).into()),
            ],
        ),
        Event::Login(l) => text(loc, "audit-login", &[("name", l.name.clone().into())]),
        Event::MessageSent(m) => {
            let what = text(
                loc,
                match m.purpose {
                    MessagePurpose::Modlog { .. } => "audit-msg-modlog",
                    MessagePurpose::OwnerDm { .. } => "audit-msg-dm",
                    MessagePurpose::Digest { .. } => "audit-msg-digest",
                },
                &[],
            );
            if m.ok {
                text(loc, "audit-message-sent", &[("what", what.into())])
            } else {
                text(
                    loc,
                    "audit-message-failed",
                    &[
                        ("what", what.into()),
                        ("error", m.error.clone().unwrap_or_default().into()),
                    ],
                )
            }
        }
        Event::ImportDone(i) => text(
            loc,
            "audit-import",
            &[
                ("from", i.from.clone().into()),
                ("sentences", i.sentences.into()),
                ("recordings", i.recordings.into()),
                ("clips", i.clips.into()),
                ("settings", i.settings.into()),
            ],
        ),
        Event::LogRepaired(r) => text(
            loc,
            "audit-log-repaired",
            &[
                ("bytes", fmt::bytes(r.cut_bytes).into()),
                ("seq", r.last_good_seq.into()),
            ],
        ),
        Event::Started(s) => text(loc, "audit-started", &[("version", s.version.clone().into())]),
        Event::Stopped(s) => text(
            loc,
            if s.clean {
                "audit-stopped"
            } else {
                "audit-stopped-unclean"
            },
            &[],
        ),
        other => text(loc, "audit-unknown", &[("kind", other.kind().to_owned().into())]),
    }
}

#[component]
pub fn AuditPage() -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let q = use_query_map();
    let group = q
        .with_untracked(|q| q.get("kind"))
        .and_then(|k| GROUPS.iter().find(|(g, _)| *g == k))
        .map(|(g, kinds)| (*g, *kinds));
    let chosen = q
        .with_untracked(|q| q.get("community"))
        .and_then(|g| g.parse::<u64>().ok())
        .map(GuildId)
        .filter(|g| v.may_see(*g));
    let filter = AuditFilter {
        guilds: chosen.map(|g| vec![g]).or_else(|| visible(&v)),
        kinds: group
            .map(|(_, kinds)| kinds.iter().map(|k| (*k).to_owned()).collect())
            .unwrap_or_default(),
        ..AuditFilter::default()
    };
    let mut communities: Vec<(GuildId, String)> = app()
        .engine
        .guilds()
        .available()
        .into_iter()
        .filter(|g| v.may_see(*g))
        .map(|g| (g, community(g)))
        .collect();
    communities.sort_by(|a, b| a.1.cmp(&b.1));
    let query = format!(
        "{}{}",
        group.map(|(g, _)| format!("&kind={g}")).unwrap_or_default(),
        chosen.map(|g| format!("&community={g}")).unwrap_or_default()
    );
    let cursor = cursor_from_query();
    let index = app().index;
    view! {
        <header class="page-head"><h1>{text(loc, "ui-nav-audit", &[])}</h1></header>
        <form method="get" action="/audit" class="row">
            <select name="kind">
                <option value="">{text(loc, "ui-all-kinds", &[])}</option>
                {GROUPS.iter().map(|(g, _)| {
                    let sel = group.is_some_and(|(x, _)| x == *g);
                    view! { <option value=*g selected=sel>{text(loc, &format!("audit-group-{g}"), &[])}</option> }
                }).collect_view()}
            </select>
            <select name="community">
                <option value="">{text(loc, "ui-all-communities", &[])}</option>
                {communities.into_iter().map(|(g, name)| {
                    let sel = chosen == Some(g);
                    view! { <option value=g.to_string() selected=sel>{name}</option> }
                }).collect_view()}
            </select>
            <button class="button">{text(loc, "ui-filter", &[])}</button>
        </form>
        <section class="card">
            <Suspense fallback=|| ()>
                {Suspend::new(async move {
                    let page = match index.audit(&filter, cursor, 50).await {
                        Err(e) => return view! { <p class="notice error">{fmt::store_error(loc, &e)}</p> }.into_any(),
                        Ok(p) if p.items.is_empty() => {
                            return view! { <p class="muted">{text(loc, "ui-nothing-yet", &[])}</p> }.into_any();
                        }
                        Ok(p) => p,
                    };
                    view! {
                        <table class="rows">
                            <tbody>
                                {page.items.into_iter().map(|r| view! {
                                    <tr>
                                        <td class="nowrap">{when(r.ts)}</td>
                                        <td>{summary(loc, &r.event)}</td>
                                    </tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                        {page.next.map(|c| view! {
                            <p><a class="button" href=format!("/audit?before={}{query}", c.0)>{text(loc, "ui-older", &[])}</a></p>
                        })}
                    }
                    .into_any()
                })}
            </Suspense>
        </section>
    }
    .into_any()
}
