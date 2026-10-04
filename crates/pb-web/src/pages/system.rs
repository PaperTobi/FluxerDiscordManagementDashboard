//! `/system` (bot owners): status, Fluxer and secrets, the global settings.

use leptos::prelude::*;
use pb_domain::Scope;
use pb_i18n::text;
use pb_live::CellSource;
use pb_live_proto::{Topic, TopicState};

use super::NotFound;
use super::settings::SectionCard;
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
                            // `de_DE`: the language by its name, the region as Piper writes it.
                            <td>{crate::fmt::language(&x.language.split(['_', '-']).next().unwrap_or_default().to_lowercase())}</td>
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
    let paused = engine.settings().current().paused_everywhere();
    view! {
        <header class="page-head"><h1>{t("ui-nav-system")}</h1></header>
        <section class="card pause-everywhere" id="pause" class:paused=paused>
            <form method="post" action="/settings" class="row">
                <input type="hidden" name="csrf" value=v.csrf.clone()/>
                <input type="hidden" name="back" value="/system"/>
                <input type="hidden" name="scope" value="global"/>
                <input type="hidden" name="key" value="paused"/>
                <input type="hidden" name="value" value=if paused { "off" } else { "on" }/>
                <input type="hidden" name="action" value="set"/>
                <div class="grow">
                    <h2>{t(if paused { "ui-paused-everywhere" } else { "ui-pause-everywhere-title" })}</h2>
                    <p class="muted small">{t(if paused { "ui-resume-everywhere-help" } else { "ui-pause-everywhere-help" })}</p>
                </div>
                <button class=if paused { "button primary" } else { "button danger" }>
                    {t(if paused { "ui-resume-everywhere" } else { "ui-pause-everywhere" })}
                </button>
            </form>
        </section>
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
        <section class="card">
            <p>{t("ui-settings-moved")} " " <a href="/settings">{t("ui-settings-global-title")}</a></p>
        </section>
        <SectionCard scope=Scope::Global section=pb_settings::Section::System back="/system".to_string()/>
    }
    .into_any()
}
