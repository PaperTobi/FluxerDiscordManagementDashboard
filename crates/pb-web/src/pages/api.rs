//! `/system/api` (bot owners): the read API's tokens (make one, see what each may read, end one) and where the API is
//! described.

use leptos::prelude::*;
use pb_i18n::text;

use super::NotFound;
use crate::app::{app, viewer};
use crate::fmt;

/// What a token may read, as the form offers it (`pb_api_proto::v1::Scope` names).
const SCOPES: [&str; 4] = ["leaderboard", "violations", "details", "stats"];

#[component]
pub fn ApiPage() -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    if !v.owner {
        return view! { <NotFound/> }.into_any();
    }
    let loc = v.locale;
    let t = move |id: &str| text(loc, id, &[]);
    let tokens = app().host.api_tokens();
    let communities: Vec<(String, String)> = {
        let gs = app().engine.guilds();
        let mut out: Vec<(String, String)> = gs
            .available()
            .into_iter()
            .map(|g| (g.to_string(), gs.guild_name(g)))
            .collect();
        out.sort_by_key(|(_, n)| n.to_lowercase());
        out
    };
    let names = communities.clone();
    let name_of = move |id: &str| {
        names
            .iter()
            .find(|(g, _)| g == id)
            .map_or_else(|| id.to_owned(), |(_, n)| n.clone())
    };
    view! {
        <h1>{t("ui-api")}</h1>
        <p class="muted">{t("ui-api-help")} " " <a href="/api/v1/openapi.json">"/api/v1/openapi.json"</a></p>
        <section class="card">
            <h2>{t("ui-api-tokens")}</h2>
            {if tokens.is_empty() {
                view! { <p class="empty">{t("ui-api-none")}</p> }.into_any()
            } else {
                view! {
                    <table class="rows">
                        <thead><tr>
                            <th>{t("ui-api-name")}</th><th>{t("ui-api-scopes")}</th><th>{t("ui-api-communities")}</th>
                            <th>{t("ui-api-last-used")}</th><th></th>
                        </tr></thead>
                        <tbody>
                            {tokens.into_iter().map(|tk| {
                                let csrf = v.csrf.clone();
                                let scopes = tk.scopes.iter().map(|s| text(loc, &format!("ui-api-scope-{s}"), &[])).collect::<Vec<_>>().join(", ");
                                let where_ = if tk.communities.is_empty() {
                                    t("ui-api-all-communities")
                                } else {
                                    tk.communities.iter().map(|g| name_of(&g.to_string())).collect::<Vec<_>>().join(", ")
                                };
                                let now = jiff::Timestamp::now().as_millisecond();
                                let used = tk.last_used_ms.map_or_else(|| t("ui-api-never"), |at| fmt::ago(loc, now, at));
                                view! {
                                    <tr>
                                        <td><b>{tk.name.clone()}</b> " " <code class="muted">{tk.id.clone()}</code></td>
                                        <td>{scopes}</td>
                                        <td>{where_}</td>
                                        <td class="muted">{used}</td>
                                        <td>
                                            <form method="post" action="/system/api/revoke" class="inline">
                                                <input type="hidden" name="csrf" value=csrf/>
                                                <input type="hidden" name="back" value="/system/api"/>
                                                <input type="hidden" name="id" value=tk.id.clone()/>
                                                <button class="button danger">{t("ui-api-revoke")}</button>
                                            </form>
                                        </td>
                                    </tr>
                                }
                            }).collect_view()}
                        </tbody>
                    </table>
                }.into_any()
            }}
        </section>
        <section class="card">
            <h2>{t("ui-api-new")}</h2>
            <form method="post" action="/system/api/create" class="stack">
                <input type="hidden" name="csrf" value=v.csrf.clone()/>
                <input type="hidden" name="back" value="/system/api"/>
                <label>{t("ui-api-name")}<input name="name" type="text" required placeholder="Leaderboard site"/></label>
                <fieldset>
                    <legend>{t("ui-api-scopes")}</legend>
                    {SCOPES.into_iter().map(|s| {
                        let on = s == "leaderboard";
                        view! {
                        <label class="inline">
                            <input type="checkbox" name="scope" value=s checked=on/>
                            {text(loc, &format!("ui-api-scope-{s}"), &[])}
                            <span class="muted small">{text(loc, &format!("ui-api-scope-{s}-help"), &[])}</span>
                        </label>
                        }
                    }).collect_view()}
                </fieldset>
                <fieldset>
                    <legend>{t("ui-api-communities")}</legend>
                    <p class="muted small">{t("ui-api-communities-help")}</p>
                    {communities.into_iter().map(|(id, name)| view! {
                        <label class="inline"><input type="checkbox" name="community" value=id/>{name}</label>
                    }).collect_view()}
                </fieldset>
                <button class="button primary">{t("ui-api-create")}</button>
            </form>
        </section>
    }
    .into_any()
}
