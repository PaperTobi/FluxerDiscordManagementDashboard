//! Server-rendered pages.

pub mod audit;
pub mod community;
pub mod person;
pub mod reports;
pub mod sentences;
pub mod settings;
pub mod setup;
pub mod system;
pub mod voicelines;
pub mod wall;

use leptos::prelude::*;

use crate::app::{NoticeBar, t};

/// "Not found" (and status 404).
#[component]
pub fn NotFound() -> impl IntoView {
    if let Some(r) = use_context::<leptos_axum::ResponseOptions>() {
        r.set_status(http::StatusCode::NOT_FOUND);
    }
    view! {
        <div class="center">
            <h1>"404"</h1>
            <p class="muted">{t("ui-not-found")}</p>
            <a class="button" href="/">{t("ui-nav-live")}</a>
        </div>
    }
}

/// A row of tabs: (link, name, current).
#[component]
pub fn Tabs(items: Vec<(String, String, bool)>) -> impl IntoView {
    view! {
        <nav class="tabs">
            {items.into_iter().map(|(href, name, active)| view! { <a href=href class:active=active>{name}</a> }).collect_view()}
        </nav>
    }
}

/// Shown instead of a page to someone who is not logged in.
#[component]
pub fn LoginNeeded() -> impl IntoView {
    view! {
        <main class="center login">
            <h1>"Profanity Watch"</h1>
            <NoticeBar/>
            <a class="button primary" href="/login">{t("ui-log-in")}</a>
            <crate::app::SourceLink/>
        </main>
    }
}
