//! `/reports`, and the swear jar the community page reuses.

use leptos::prelude::*;
use pb_domain::GuildId;
use pb_i18n::text;
use pb_store_api::{JarRow, SentenceKind};

use super::sentences::{SentenceList, here};
use crate::app::{app, viewer};

/// The swear jar (of a community, or of every community the viewer may see).
#[component]
pub fn Jar(guild: Option<GuildId>) -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let index = app().index;
    let engine = app().engine;
    let back = here();
    view! {
        <section class="card">
            <h2>{text(loc, "ui-jar", &[])}</h2>
            <Suspense fallback=|| ()>
                {Suspend::new(async move {
                    let rows: Vec<JarRow> = index
                        .jar(guild)
                        .await
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|r| v.may_see(r.guild) && r.count > 0)
                        .collect();
                    if rows.is_empty() {
                        return view! { <p class="muted">{text(loc, "ui-jar-empty", &[])}</p> }.into_any();
                    }
                    let gs = engine.guilds();
                    view! {
                        <table class="rows">
                            <tbody>
                                {rows.into_iter().map(|r| view! {
                                    <tr>
                                        <td>
                                            <a href=format!("/c/{}/p/{}", r.guild, r.user)>{gs.name(r.guild, r.user)}</a>
                                            {guild.is_none().then(|| view! { <span class="muted small">" · " {gs.guild_name(r.guild)}</span> })}
                                        </td>
                                        <td class="num"><b>{r.count}</b></td>
                                        <td class="right">
                                            <form method="post" action="/jar/reset">
                                                <input type="hidden" name="csrf" value=v.csrf.clone()/>
                                                <input type="hidden" name="back" value=back.clone()/>
                                                <input type="hidden" name="guild" value=r.guild.to_string()/>
                                                <input type="hidden" name="user" value=r.user.to_string()/>
                                                <button class="link">{text(loc, "ui-jar-reset", &[])}</button>
                                            </form>
                                        </td>
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

/// `/reports`
#[component]
pub fn ReportsPage() -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let owner = v.owner;
    let index = app().index;
    view! {
        <header class="page-head"><h1>{text(loc, "ui-nav-reports", &[])}</h1></header>
        {owner.then(|| view! {
            <section class="card">
                <h2>{text(loc, "ui-digest", &[])}</h2>
                <p class="muted">{text(loc, "ui-digest-help", &[])}</p>
                <Suspense fallback=|| ()>
                    {Suspend::new(async move {
                        let last = index.last_digest().await.unwrap_or_default();
                        match (last.error, last.sent) {
                            (Some(e), _) => Some(view! {
                                <p class="notice error">{text(loc, "ui-digest-failed", &[("error", e.into())])}</p>
                            }.into_any()),
                            (None, Some(t)) => Some(view! {
                                <p>{text(loc, "ui-digest-last-sent", &[("when", super::sentences::when(t).into())])}</p>
                            }.into_any()),
                            (None, None) => None,
                        }
                    })}
                </Suspense>
                <form method="post" action="/reports/send">
                    <input type="hidden" name="csrf" value=v.csrf.clone()/>
                    <input type="hidden" name="back" value="/reports"/>
                    <button class="button">{text(loc, "ui-digest-send", &[])}</button>
                </form>
            </section>
        })}
        <Jar guild=None/>
        <SentenceList guild=None user=None kind=SentenceKind::Violations/>
    }
    .into_any()
}
