//! `/setup`: the first-start wizard (setup code → instance → bot token → client secret → owner login). Each step is a
//! plain form; the server keeps the progress.

use leptos::prelude::*;
use pb_i18n::text;

use crate::app::{NoticeBar, SetupStep, app, locale};

/// The steps: what they are called, and their name when opened again (the code cannot be).
const STEPS: [(SetupStep, &str, Option<&str>); 5] = [
    (SetupStep::Code, "setup-step-code", None),
    (SetupStep::Instance, "setup-step-instance", Some("instance")),
    (SetupStep::Token, "setup-step-token", Some("token")),
    (SetupStep::ClientSecret, "setup-step-secret", Some("secret")),
    (SetupStep::Owner, "setup-step-owner", None),
];

/// A small form that opens a done step again (`goto`) or goes on without changing the one shown (`keep`).
#[component]
fn Nav(step: &'static str, value: &'static str, csrf: String, label: String) -> impl IntoView {
    view! {
        <form method="post" action="/setup" class="inline">
            <input type="hidden" name="step" value=step/>
            <input type="hidden" name="csrf" value=csrf/>
            <input type="hidden" name="value" value=value/>
            <button class="button">{label}</button>
        </form>
    }
}

/// One step's form: a hidden step name and token, one field, a button.
#[component]
fn StepForm(step: &'static str, csrf: String, children: Children) -> impl IntoView {
    let loc = locale();
    view! {
        <form method="post" action="/setup" class="stack">
            <input type="hidden" name="step" value=step/>
            <input type="hidden" name="csrf" value=csrf/>
            {children()}
            <button class="button primary">{text(loc, "setup-continue", &[])}</button>
        </form>
    }
}

#[component]
pub fn SetupPage() -> impl IntoView {
    let loc = locale();
    let Some(parts) = use_context::<http::request::Parts>() else {
        return ().into_any();
    };
    let s = app().host.setup(&parts);
    let tt = move |id: &str| text(loc, id, &[]);
    // Back to the step before (from the bot token on); a step opened again can also be kept as it is.
    let previous = STEPS
        .iter()
        .take_while(|(st, _, _)| *st < s.step)
        .filter_map(|(_, _, name)| *name)
        .last()
        .filter(|_| s.step > SetupStep::Instance && s.step != SetupStep::Done);
    let nav = view! {
        <div class="actions">
            {previous.map(|p| view! { <Nav step="goto" value=p csrf=s.csrf.clone() label=tt("setup-back")/> })}
            {(s.step < s.reached).then(|| view! { <Nav step="keep" value="" csrf=s.csrf.clone() label=tt("setup-keep")/> })}
        </div>
    };
    let body = match s.step {
        SetupStep::Done => view! {
            <p>{tt("setup-done")}</p>
            <a class="button primary" href="/">{tt("setup-open")}</a>
        }
        .into_any(),
        SetupStep::Code => view! {
            <p>{text(loc, "setup-code-help", &[("file", s.code_file.clone().into())])}</p>
            <form method="post" action="/setup" class="stack">
                <input type="hidden" name="step" value="code"/>
                <label>
                    {tt("setup-code-label")}
                    <input name="value" autocomplete="off" autocapitalize="characters" spellcheck="false" required
                        autofocus/>
                </label>
                {(s.wait_secs > 0).then(|| view! {
                    <p class="muted">{text(loc, "setup-code-wait", &[("s", s.wait_secs.into())])}</p>
                })}
                <button class="button primary">{tt("setup-continue")}</button>
            </form>
        }
        .into_any(),
        SetupStep::Instance => view! {
            <p>{tt("setup-instance-help")}</p>
            <StepForm step="instance" csrf=s.csrf.clone()>
                <label>
                    {tt("setup-instance-label")}
                    <input name="value" type="text" inputmode="url" spellcheck="false" autocomplete="off"
                        value=s.instance.clone() placeholder="https://api.fluxer.app" required autofocus/>
                </label>
            </StepForm>
        }
        .into_any(),
        SetupStep::Token => view! {
            <p>{tt("setup-token-help")}</p>
            {s.has_token.then(|| view! { <p class="muted">{tt("setup-token-saved")}</p> })}
            <StepForm step="token" csrf=s.csrf.clone()>
                <label>
                    {tt("setup-token-label")}
                    <input name="value" type="password" autocomplete="off" required autofocus/>
                </label>
            </StepForm>
        }
        .into_any(),
        SetupStep::ClientSecret => view! {
            <p>{tt("setup-secret-help")}</p>
            <p class="copy"><code>{s.redirect_uri.clone()}</code></p>
            <p class="muted">{tt("setup-secret-address")}</p>
            {s.has_secret.then(|| view! { <p class="muted">{tt("setup-secret-saved")}</p> })}
            <StepForm step="secret" csrf=s.csrf.clone()>
                <label>
                    {tt("setup-secret-label")}
                    <input name="value" type="password" autocomplete="off" required autofocus/>
                </label>
            </StepForm>
        }
        .into_any(),
        SetupStep::Owner => view! {
            {s.bot.clone().map(|b| view! { <p>{text(loc, "setup-bot-ready", &[("bot", b.into())])}</p> })}
            <p>{tt("setup-owner-help")}</p>
            <a class="button primary" href="/login">{tt("ui-log-in")}</a>
            // A login that fails at Fluxer comes back here: the address and the secret can be corrected.
            <details>
                <summary>{tt("setup-owner-trouble")}</summary>
                <p>{tt("setup-secret-help")}</p>
                <p class="copy"><code>{s.redirect_uri.clone()}</code></p>
                {if s.secret_from_env {
                    view! { <p class="muted">{tt("ui-secret-from-env")}</p> }.into_any()
                } else {
                    view! {
                        <StepForm step="secret" csrf=s.csrf.clone()>
                            <label>
                                {tt("setup-secret-again")}
                                <input name="value" type="password" autocomplete="off" required/>
                            </label>
                        </StepForm>
                    }
                    .into_any()
                }}
            </details>
        }
        .into_any(),
    };
    let (current, reached, csrf) = (s.step, s.reached, s.csrf.clone());
    view! {
        <main class="center setup">
            <h1>"Profanity Watch"</h1>
            <ol class="steps">
                {STEPS
                    .iter()
                    .map(|(st, id, name)| {
                        // A done step (before the furthest reached) can be opened again.
                        let open = name.filter(|_| *st < reached && *st != current && reached != SetupStep::Done);
                        view! {
                            <li class:done=(*st < reached) class:current=(*st == current)>
                                {match open {
                                    Some(n) => view! {
                                        <form method="post" action="/setup" class="inline">
                                            <input type="hidden" name="step" value="goto"/>
                                            <input type="hidden" name="csrf" value=csrf.clone()/>
                                            <input type="hidden" name="value" value=n/>
                                            <button class="link" title=tt("setup-change-step")>{tt(id)}</button>
                                        </form>
                                    }
                                    .into_any(),
                                    None => tt(id).into_any(),
                                }}
                            </li>
                        }
                    })
                    .collect_view()}
            </ol>
            <NoticeBar/>
            {body}
            {nav}
        </main>
    }
    .into_any()
}
