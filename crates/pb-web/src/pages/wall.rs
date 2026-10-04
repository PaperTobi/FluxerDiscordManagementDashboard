//! `/`: the live wall, with a first step when there is nothing to show yet (no community: invite the bot; nobody
//! tracked: how to track someone).

use leptos::prelude::*;
use pb_i18n::text;
use pb_live_proto::{Topic, TopicState, WallState};

use crate::app::{app, viewer};
use crate::islands::WallLive;

#[component]
pub fn WallPage() -> impl IntoView {
    let v = viewer();
    let loc = crate::app::locale();
    let engine = app().engine;
    let mut wall = match engine.hub().state(&Topic::Wall) {
        Some(TopicState::Wall(w)) => w,
        _ => WallState::default(),
    };
    let mut communities = match engine.hub().state(&Topic::Sidebar) {
        Some(TopicState::Sidebar(s)) => s.communities,
        _ => Vec::new(),
    };
    if let Some(v) = v.filter(|v| !v.owner) {
        wall.tiles.retain(|t| v.guilds.contains(&t.guild));
        wall.violations.retain(|x| v.guilds.contains(&x.guild));
        communities.retain(|c| v.guilds.contains(&c.id));
    }
    let first_step = if communities.is_empty() {
        Some(
            view! {
                <section class="card first-step">
                    <p>{text(loc, "ui-first-invite", &[])}</p>
                    <a class="button primary" href="/invite">{text(loc, "ui-invite-title", &[])}</a>
                </section>
            }
            .into_any(),
        )
    } else if communities.iter().all(|c| c.people.is_empty()) {
        let first = communities[0].id;
        Some(
            view! {
                <section class="card first-step">
                    <p>{text(loc, "ui-first-track", &[])}</p>
                    <a class="button primary" href=format!("/c/{first}")>{text(loc, "ui-track-someone", &[])}</a>
                </section>
            }
            .into_any(),
        )
    } else {
        None
    };
    view! {
        <h1>{text(loc, "ui-wall-title", &[])}</h1>
        {first_step}
        <WallLive initial=wall locale=loc/>
    }
}
