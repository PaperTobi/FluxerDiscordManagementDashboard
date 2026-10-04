//! `/system` (bot owners): status, Fluxer and secrets, the global settings.

use leptos::prelude::*;
use pb_domain::Scope;
use pb_i18n::text;
use pb_live::CellSource;
use pb_live_proto::{Topic, TopicState};

use super::NotFound;
use super::settings::SettingsForm;
use crate::app::{app, viewer};
use crate::islands::SystemLive;

/// The text-to-speech voices, and how to add more.
#[component]
fn Voices() -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let mut voices = app().engine.voices();
    voices.sort_by(|a, b| a.language.cmp(&b.language).then(a.id.cmp(&b.id)));
    view! {
        <section class="card">
            <h2>{text(loc, "ui-voices", &[])}</h2>
            <table class="rows">
                <tbody>
                    {voices.into_iter().map(|x| view! {
                        <tr>
                            <td>{crate::fmt::language(&x.language)}</td>
                            <td><code>{x.id}</code></td>
                            <td class="muted">{x.quality}</td>
                        </tr>
                    }).collect_view()}
                </tbody>
            </table>
            <p class="muted small">{text(loc, "ui-voices-help", &[])}</p>
        </section>
    }
    .into_any()
}

#[component]
pub fn SystemPage() -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    if !v.owner {
        return view! { <NotFound/> }.into_any();
    }
    let loc = v.locale;
    let engine = app().engine;
    engine.cells().ensure(&Topic::System);
    let status = match engine.hub().state(&Topic::System) {
        Some(TopicState::System(s)) => Some(*s),
        _ => None,
    };
    let t = move |id: &str| text(loc, id, &[]);
    let (token_env, secret_env) = app().host.secrets_from_env();
    let secret_form = |action: &'static str, label: &'static str, help: &'static str, from_env: bool| {
        if from_env {
            return view! {
                <div class="stack">
                    <b>{t(label)}</b>
                    <p class="muted small">{t("ui-secret-from-env")}</p>
                </div>
            }
            .into_any();
        }
        view! {
            <form method="post" action=action class="stack">
                <input type="hidden" name="csrf" value=v.csrf.clone()/>
                <input type="hidden" name="back" value="/system"/>
                <label>{t(label)}<input name="value" type="password" autocomplete="off" required/></label>
                <p class="muted small">{t(help)}</p>
                <button class="button">{t("ui-replace")}</button>
            </form>
        }
        .into_any()
    };
    view! {
        <header class="page-head"><h1>{t("ui-nav-system")}</h1></header>
        {status.map(|s| view! { <SystemLive initial=s locale=loc csrf=v.csrf.clone()/> })}
        <section class="card">
            <h2>{t("ui-fluxer-secrets")}</h2>
            <p class="muted">{t("ui-secrets-help")}</p>
            <div class="grid two">
                {secret_form("/system/token", "setup-token-label", "setup-token-help", token_env)}
                {secret_form("/system/client-secret", "setup-secret-label", "ui-client-secret-help", secret_env)}
            </div>
            <div class="row">
                <form method="post" action="/system/reconnect">
                    <input type="hidden" name="csrf" value=v.csrf.clone()/>
                    <input type="hidden" name="back" value="/system"/>
                    <button class="button">{t("ui-reconnect")}</button>
                </form>
                <form method="post" action="/system/reload">
                    <input type="hidden" name="csrf" value=v.csrf.clone()/>
                    <input type="hidden" name="back" value="/system"/>
                    <button class="button">{t("ui-reload-settings")}</button>
                </form>
            </div>
        </section>
        <Voices/>
        <SettingsForm scope=Scope::Global back="/system".to_string()/>
    }
    .into_any()
}
