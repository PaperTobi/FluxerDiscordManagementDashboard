//! `/setup`: the first-start wizard (setup code → instance → bot token → client secret → owner login). Each step is a
//! plain form; the server keeps the progress.

use leptos::prelude::*;
use pb_i18n::text;

use crate::app::{NoticeBar, SetupStep, app, locale};

const STEPS: [(SetupStep, &str); 5] = [
    (SetupStep::Code, "setup-step-code"),
    (SetupStep::Instance, "setup-step-instance"),
    (SetupStep::Token, "setup-step-token"),
    (SetupStep::ClientSecret, "setup-step-secret"),
    (SetupStep::Owner, "setup-step-owner"),
];

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
                    <input name="value" type="url" value=s.instance.clone() required autofocus/>
                </label>
            </StepForm>
        }
        .into_any(),
        SetupStep::Token => view! {
            <p>{tt("setup-token-help")}</p>
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
        }
        .into_any(),
    };
    let current = s.step;
    view! {
        <main class="center setup">
            <h1>"Profanity Watch"</h1>
            <ol class="steps">
                {STEPS
                    .iter()
                    .map(|(st, id)| {
                        view! { <li class:done=(*st < current) class:current=(*st == current)>{tt(id)}</li> }
                    })
                    .collect_view()}
            </ol>
            <NoticeBar/>
            {body}
        </main>
    }
    .into_any()
}
