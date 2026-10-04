//! The live wall: one tile per person the bot hears, and the latest violations.

use leptos::prelude::*;
use pb_i18n::{Locale, text};
use pb_live_proto::{SentenceCard, Station, Tile, Topic, TopicState, WallState, display_stage};

use super::widgets::{Avatar, Sparkline};
use crate::fmt;

#[island]
pub fn WallLive(initial: WallState, locale: Locale) -> impl IntoView {
    let state = RwSignal::new(initial);
    let w = crate::live::watch(Topic::Wall, move |s| {
        if let TopicState::Wall(s) = s {
            state.set(s.clone());
        }
    });
    on_cleanup(move || w.stop());
    // The conveyor's clock: stations follow the bot's time.
    let now = RwSignal::new(crate::live::server_now());
    // Browser only: the server renders one picture and has no timers (calling the browser there aborts).
    #[cfg(feature = "hydrate")]
    {
        let tick = set_interval_with_handle(
            move || now.set(crate::live::server_now()),
            std::time::Duration::from_millis(250),
        )
        .ok();
        on_cleanup(move || {
            if let Some(t) = tick {
                t.clear();
            }
        });
    }
    let tiles = move || state.get().tiles;
    view! {
        <section class="wall">
            <Show when=move || !state.with(|s| s.tiles.is_empty()) fallback=move || view! { <p class="empty">{text(locale, "ui-wall-empty", &[])}</p> }>
                <div class="tiles">
                    <For each=tiles key=|t| (t.guild, t.who.user, t.channel.id) let:t>
                        <TileView tile=t state now locale/>
                    </For>
                </div>
            </Show>
        </section>
        <section class="card">
            <h2>{text(locale, "ui-wall-violations", &[])}</h2>
            <Show when=move || !state.with(|s| s.violations.is_empty()) fallback=move || view! { <p class="muted">{text(locale, "ui-wall-no-violations", &[])}</p> }>
                <ul class="feed">
                    <For each=move || state.get().violations key=|v| v.sentence let:v>
                        <li>
                            <a href=format!("/c/{}/p/{}", v.guild, v.who.user)>{v.who.name.clone()}</a>
                            " · " {fmt::label(locale, v.label)} " " <b>{fmt::pct(v.score)}</b>
                            " · " <span class=format!("chip {}", fmt::decision_class(v.decision))>{fmt::decision(locale, v.decision)}</span>
                            <span class="when">{move || fmt::ago(locale, now.get(), v.at_ms)}</span>
                        </li>
                    </For>
                </ul>
            </Show>
        </section>
    }
}

#[component]
fn TileView(tile: Tile, state: RwSignal<WallState>, now: RwSignal<i64>, locale: Locale) -> impl IntoView {
    let key = (tile.guild, tile.who.user);
    let current = Memo::new(move |_| state.with(|s| s.tiles.iter().find(|t| (t.guild, t.who.user) == key).cloned()));
    let levels = Signal::derive(move || current.get().map(|t| t.levels).unwrap_or_default());
    let latest = move || current.get().and_then(|t| t.latest);
    let href = format!("/c/{}/p/{}", tile.guild, tile.who.user);
    view! {
        <a class="tile" href=href data-user=tile.who.user.to_string()>
            <header>
                <Avatar who=tile.who.clone() size=36/>
                <div>
                    <div class="name">{tile.who.name.clone()}</div>
                    <div class="where">{tile.community.clone()} " · " {tile.channel.name.clone()}</div>
                </div>
            </header>
            <Sparkline levels/>
            <footer>
                {move || latest().map(|c| view! { <Card card=c now locale/> })}
                {move || current.get().and_then(|t| t.lag_ms).map(|l| view! {
                    <span class="lag">{text(locale, "ui-lag", &[("ms", fmt::ms(locale, u64::from(l)).into())])}</span>
                })}
            </footer>
        </a>
    }
}

/// The latest sentence: where it is on the conveyor, and the verdict once there is one.
#[component]
fn Card(card: SentenceCard, now: RwSignal<i64>, locale: Locale) -> impl IntoView {
    let stamps = card.stamps;
    let shown = move || display_stage(&stamps, now.get());
    let verdict = card.verdict.clone();
    let decision = card.decision;
    view! {
        <span class="station" data-card=card.id.to_string() data-station=move || format!("{:?}", shown().station).to_lowercase()>
            {move || fmt::station(locale, shown().station)}
        </span>
        {move || {
            let s = shown();
            (s.station == Station::Decision).then(|| {
                let top = verdict.as_ref().map(|v| (v.top(), v.score(v.top())));
                view! {
                    {top.map(|(l, sc)| view! { <span class="top">{fmt::label(locale, l)} " " {fmt::pct(sc)}</span> })}
                    {decision.map(|d| view! { <span class=format!("chip {}", fmt::decision_class(d))>{fmt::decision(locale, d)}</span> })}
                }
            })
        }}
    }
}
