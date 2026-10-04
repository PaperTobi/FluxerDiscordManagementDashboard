//! Lists of sentences (violations, a person's history, recordings), newest first and paged, with each sentence's
//! scores against the bars it had to reach. Shared by the reports, community and person pages.

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use pb_domain::{GuildId, Label, UserId};
use pb_i18n::{Locale, text};
use pb_store_api::{Cursor, SentenceFilter, SentenceKind, SentenceRecord};

use crate::app::{Viewer, app, viewer};
use crate::fmt;

/// Rows per page.
const PAGE: u32 = 50;

/// A time in the reporting time zone.
pub fn when(ts: jiff::Timestamp) -> String {
    let zone = app()
        .engine
        .settings()
        .current()
        .effective(None, None)
        .timezone
        .value
        .zone();
    ts.to_zoned(zone).strftime("%Y-%m-%d %H:%M").to_string()
}

/// The communities this viewer may see (`None` = all: the owner).
pub fn visible(v: &Viewer) -> Option<Vec<GuildId>> {
    (!v.owner).then(|| v.guilds.iter().copied().collect())
}

/// The `before` cursor of a paged list.
pub fn cursor_from_query() -> Option<Cursor> {
    use_query_map()
        .with_untracked(|q| q.get("before"))
        .and_then(|s| s.parse().ok())
        .map(Cursor)
}

/// The path of the page being rendered (forms come back to it).
pub fn here() -> String {
    use_context::<http::request::Parts>()
        .map(|p| p.uri.path().to_owned())
        .unwrap_or_default()
}

/// Sentences of `kind` in a community (or every community the viewer may see), of one person or everyone.
#[component]
pub fn SentenceList(guild: Option<GuildId>, user: Option<UserId>, kind: SentenceKind) -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let f = SentenceFilter {
        guilds: guild.map(|g| vec![g]).or_else(|| visible(&v)),
        user,
        kind,
        ..SentenceFilter::default()
    };
    let cursor = cursor_from_query();
    let path = here();
    let (title, empty) = match kind {
        SentenceKind::Violations => ("ui-violations", "ui-wall-no-violations"),
        SentenceKind::WithAudio => ("ui-tab-evidence", "ui-no-evidence"),
        SentenceKind::All | SentenceKind::Flagged => ("ui-sentences", "ui-no-sentences"),
    };
    let index = app().index;
    view! {
        <section class="card">
            <h2>{text(loc, title, &[])}</h2>
            <Suspense fallback=|| ()>
                {Suspend::new(async move {
                    let page = match index.sentences(&f, cursor, PAGE).await {
                        Err(e) => return view! { <p class="notice error">{fmt::store_error(loc, &e)}</p> }.into_any(),
                        Ok(p) if p.items.is_empty() => {
                            return view! { <p class="muted">{text(loc, empty, &[])}</p> }.into_any();
                        }
                        Ok(p) => p,
                    };
                    view! {
                        <table class="rows">
                            <thead><tr>
                                <th>{text(loc, "ui-when", &[])}</th>
                                {user.is_none().then(|| view! { <th>{text(loc, "ui-who", &[])}</th> })}
                                <th>{text(loc, "ui-length", &[])}</th>
                                <th>{text(loc, "ui-scores", &[])}</th>
                                <th>{text(loc, "ui-decision", &[])}</th>
                                <th></th>
                            </tr></thead>
                            <tbody>
                                {page.items.into_iter().map(|r| {
                                    let audio = r.record.audio.is_some() && !r.audio_deleted;
                                    row(&v, &r.record, audio, user.is_none(), &path)
                                }).collect_view()}
                            </tbody>
                        </table>
                        {page.next.map(|c| view! {
                            <p><a class="button" href=format!("{path}?before={}", c.0)>{text(loc, "ui-older", &[])}</a></p>
                        })}
                    }
                    .into_any()
                })}
            </Suspense>
        </section>
    }
    .into_any()
}

fn row(v: &Viewer, rec: &SentenceRecord, audio: bool, show_who: bool, back: &str) -> impl IntoView + use<> {
    let loc = v.locale;
    let d = pb_engine::decision_view(&rec.decision);
    let gs = app().engine.guilds();
    let who = show_who.then(|| {
        view! {
            <td>
                <a href=format!("/c/{}/p/{}", rec.guild, rec.user)>{gs.name(rec.guild, rec.user)}</a>
                <div class="muted small">{gs.guild_name(rec.guild)} " · " {gs.channel_name(rec.guild, rec.channel)}</div>
            </td>
        }
    });
    let recording = audio.then(|| {
        let playable = v.may_play_recordings(rec.guild);
        view! {
            {if playable {
                view! { <audio controls preload="none" src=format!("/media/sentence/{}", rec.id)></audio> }.into_any()
            } else {
                view! { <span class="muted small">{text(loc, "ui-recording-kept", &[])}</span> }.into_any()
            }}
            {v.owner.then(|| view! {
                <form method="post" action="/evidence/delete" class="inline">
                    <input type="hidden" name="csrf" value=v.csrf.clone()/>
                    <input type="hidden" name="back" value=back.to_owned()/>
                    <input type="hidden" name="sentence" value=rec.id.to_string()/>
                    <button class="link danger">{text(loc, "ui-delete-recording", &[])}</button>
                </form>
            })}
        }
    });
    view! {
        <tr>
            <td class="nowrap">{when(rec.started)}</td>
            {who}
            <td class="num">{fmt::ms(loc, u64::from(rec.dur_ms))}</td>
            <td>{scores(loc, rec)}</td>
            <td><span class=format!("chip {}", fmt::decision_class(d))>{fmt::decision(loc, d)}</span></td>
            <td>{recording}</td>
        </tr>
    }
}

/// The highest score (or the flagged type) at a glance; every type against its bar when opened.
fn scores(loc: Locale, rec: &SentenceRecord) -> impl IntoView + use<> {
    let top = rec.flagged.first().copied().unwrap_or_else(|| {
        Label::ALL
            .iter()
            .copied()
            .max_by(|a, b| rec.scores[a.index()].total_cmp(&rec.scores[b.index()]))
            .unwrap_or(Label::Profanity)
    });
    let bar = |l: Label| rec.thresholds.iter().find(|(t, _)| *t == l).map(|(_, b)| *b);
    let rows = Label::ALL
        .iter()
        .map(|l| {
            let flagged = rec.flagged.contains(l);
            view! {
                <tr class:flagged=flagged>
                    <td>{fmt::label(loc, *l)}</td>
                    <td class="num">{fmt::pct(rec.scores[l.index()])}</td>
                    <td class="num muted">{bar(*l).map(|b| format!("≥ {}", fmt::pct(b)))}</td>
                </tr>
            }
        })
        .collect_view();
    view! {
        <details class="scores">
            <summary>{fmt::label(loc, top)} " " <b>{fmt::pct(rec.scores[top.index()])}</b></summary>
            <table class="mini">
                <tbody>{rows}</tbody>
            </table>
            <div class="muted small">{text(loc, "ui-heard-language", &[("lang", fmt::language(rec.language.code()).into())])}</div>
        </details>
    }
}
