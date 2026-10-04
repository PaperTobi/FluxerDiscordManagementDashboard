//! `/`: the live wall.

use leptos::prelude::*;
use pb_i18n::text;
use pb_live_proto::{Topic, TopicState, WallState};

use crate::app::{app, viewer};
use crate::islands::WallLive;

#[component]
pub fn WallPage() -> impl IntoView {
    let v = viewer();
    let loc = crate::app::locale();
    let mut wall = match app().engine.hub().state(&Topic::Wall) {
        Some(TopicState::Wall(w)) => w,
        _ => WallState::default(),
    };
    if let Some(v) = v.filter(|v| !v.owner) {
        wall.tiles.retain(|t| v.guilds.contains(&t.guild));
        wall.violations.retain(|x| v.guilds.contains(&x.guild));
    }
    view! {
        <h1>{text(loc, "ui-wall-title", &[])}</h1>
        <WallLive initial=wall locale=loc/>
    }
}
