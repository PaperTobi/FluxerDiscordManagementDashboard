//! `/c/:g/p/:u` and its tabs: Live, History, Evidence, Voice lines, Settings.

use leptos::prelude::*;
use leptos_router::hooks::use_params_map;
use pb_domain::{GuildId, Scope, UserId};
use pb_i18n::text;
use pb_store_api::SentenceKind;

use super::sentences::SentenceList;
use super::settings::SettingsForm;
use super::{NotFound, Tabs};
use crate::app::{Viewer, app, viewer};
use crate::fmt;
use crate::islands::{Avatar, PersonLive};

#[component]
pub fn PersonPage() -> impl IntoView {
    let params = use_params_map();
    let num = |k: &str| params.with_untracked(|p| p.get(k)).and_then(|s| s.parse::<u64>().ok());
    let (Some(g), Some(u)) = (num("g").map(GuildId), num("u").map(UserId)) else {
        return view! { <NotFound/> }.into_any();
    };
    let tab = params.with_untracked(|p| p.get("tab")).unwrap_or_default();
    let Some(v) = viewer() else {
        return ().into_any();
    };
    if !v.may_see(g) {
        return view! { <NotFound/> }.into_any();
    }
    let engine = app().engine;
    if !engine.knows_guild(g) {
        return view! { <NotFound/> }.into_any();
    }
    let who = engine.who(g, u);
    let tracking = engine.tracking(g, u);
    let loc = v.locale;
    let base = format!("/c/{g}/p/{u}");
    let here = if tab.is_empty() {
        base.clone()
    } else {
        format!("{base}/{tab}")
    };
    let tabs = [
        ("", "ui-tab-live"),
        ("history", "ui-tab-history"),
        ("evidence", "ui-tab-evidence"),
        ("voice-lines", "ui-nav-voice-lines"),
        ("settings", "ui-tab-settings"),
    ]
    .into_iter()
    .map(|(t, id)| {
        let href = if t.is_empty() {
            base.clone()
        } else {
            format!("{base}/{t}")
        };
        (href, text(loc, id, &[]), t == tab)
    })
    .collect::<Vec<_>>();
    let live_engine = engine.clone();
    let body = match tab.as_str() {
        "" => view! {
            <Suspense fallback=|| ()>
                {Suspend::new(async move {
                    live_engine
                        .person_view(g, u)
                        .await
                        .map(|s| view! { <PersonLive initial=s locale=loc/> })
                })}
            </Suspense>
            <SayNow guild=g user=u viewer=v.clone() back=here.clone()/>
        }
        .into_any(),
        "history" => view! {
            <Days guild=g user=u/>
            <SentenceList guild=Some(g) user=Some(u) kind=SentenceKind::All/>
        }
        .into_any(),
        "evidence" => view! { <SentenceList guild=Some(g) user=Some(u) kind=SentenceKind::WithAudio/> }.into_any(),
        "voice-lines" => {
            view! { <super::voicelines::VoiceLinesEditor scope=Scope::Person { guild: g, user: u } back=here.clone()/> }
                .into_any()
        }
        "settings" => view! { <SettingsForm scope=Scope::Person { guild: g, user: u } back=here.clone()/> }.into_any(),
        _ => view! { <NotFound/> }.into_any(),
    };
    let track_form = {
        let (action, label) = if tracking.listed && !tracking.everywhere {
            ("/people/untrack", "ui-untrack")
        } else {
            ("/people/track", "ui-track")
        };
        (!tracking.everywhere).then(|| {
            view! {
                <form method="post" action=action>
                    <input type="hidden" name="csrf" value=v.csrf.clone()/>
                    <input type="hidden" name="back" value=here.clone()/>
                    <input type="hidden" name="guild" value=g.to_string()/>
                    <input type="hidden" name="user" value=u.to_string()/>
                    <button class="button">{text(loc, label, &[])}</button>
                </form>
            }
        })
    };
    view! {
        <header class="page-head person-head">
            <Avatar who=who.clone() size=48/>
            <div class="grow">
                <h1>{who.name.clone()}</h1>
                <div class="muted"><a href=format!("/c/{g}")>{engine.guilds().guild_name(g)}</a></div>
            </div>
            {tracking.everywhere.then(|| view! { <span class="chip">{text(loc, "ui-everywhere", &[])}</span> })}
            {(tracking.listed && !tracking.active).then(|| view! { <span class="chip warn">{text(loc, "ui-paused", &[])}</span> })}
            {(!tracking.listed).then(|| view! { <span class="chip">{text(loc, "ui-not-tracked", &[])}</span> })}
            {track_form}
        </header>
        <Tabs items=tabs/>
        {body}
    }
    .into_any()
}

/// "Say now": a text in a language, spoken in the call this person is in (only they, or everyone, hear it as the
/// audience setting says).
#[component]
fn SayNow(guild: GuildId, user: UserId, viewer: Viewer, back: String) -> impl IntoView {
    let loc = viewer.locale;
    let langs = super::settings::language_list();
    let presets = app()
        .engine
        .settings()
        .current()
        .scoped_slots(guild, Some(user))
        .say_presets();
    view! {
        <section class="card">
            <h2>{text(loc, "ui-say-now", &[])}</h2>
            <form method="post" action="/say" class="row">
                <input type="hidden" name="csrf" value=viewer.csrf.clone()/>
                <input type="hidden" name="back" value=back/>
                <input type="hidden" name="guild" value=guild.to_string()/>
                <input type="hidden" name="user" value=user.to_string()/>
                {(!presets.is_empty()).then(|| view! {
                    <select name="preset">
                        <option value="">{text(loc, "ui-say-own-text", &[])}</option>
                        {presets.into_iter().map(|p| view! { <option value=p.clone()>{p.replace('-', " ")}</option> }).collect_view()}
                    </select>
                })}
                <input name="text" class="grow" placeholder=text(loc, "ui-say-placeholder", &[])/>
                <select name="lang">
                    <option value="">{text(loc, "ui-their-language", &[])}</option>
                    {langs.into_iter().map(|l| view! { <option value=l.clone()>{fmt::language(&l)}</option> }).collect_view()}
                </select>
                <button class="button primary">{text(loc, "ui-say", &[])}</button>
            </form>
            <p class="muted small">{text(loc, "ui-say-help", &[])}</p>
        </section>
    }
}

/// The last two weeks, per day.
#[component]
fn Days(guild: GuildId, user: UserId) -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let index = app().index;
    let tz = app()
        .engine
        .settings()
        .current()
        .effective(Some(guild), Some(user))
        .timezone
        .value;
    view! {
        <section class="card">
            <h2>{text(loc, "ui-days", &[])}</h2>
            <Suspense fallback=|| ()>
                {Suspend::new(async move {
                    let today = jiff::Timestamp::now().to_zoned(tz.zone()).date();
                    let from = today.checked_sub(jiff::Span::new().days(13)).unwrap_or(today);
                    let rows = index.days(guild, user, from, today, &tz.to_string()).await.unwrap_or_default();
                    if rows.is_empty() {
                        return view! { <p class="muted">{text(loc, "ui-no-sentences", &[])}</p> }.into_any();
                    }
                    view! {
                        <table class="rows">
                            <thead><tr>
                                <th>{text(loc, "ui-day", &[])}</th>
                                <th class="num">{text(loc, "ui-sentences", &[])}</th>
                                <th class="num">{text(loc, "ui-flagged", &[])}</th>
                                <th class="num">{text(loc, "ui-violations", &[])}</th>
                                <th class="num">{text(loc, "ui-speech", &[])}</th>
                            </tr></thead>
                            <tbody>
                                {rows.into_iter().rev().map(|d| view! {
                                    <tr>
                                        <td>{d.day.to_string()}</td>
                                        <td class="num">{d.sentences}</td>
                                        <td class="num">{d.flagged}</td>
                                        <td class="num">{d.violations}</td>
                                        <td class="num">{fmt::ms(loc, d.speech_ms)}</td>
                                    </tr>
                                }).collect_view()}
                            </tbody>
                        </table>
                    }
                    .into_any()
                })}
            </Suspense>
        </section>
    }
    .into_any()
}
