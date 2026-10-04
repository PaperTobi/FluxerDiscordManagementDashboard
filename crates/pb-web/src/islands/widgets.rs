//! Small pieces several islands use.

use leptos::prelude::*;
use pb_live_proto::{Dot, Levels, Who};

/// A person's picture, or their initial.
#[component]
pub fn Avatar(who: Who, #[prop(default = 32)] size: u32) -> impl IntoView {
    // Sizes are classes, not inline styles (the page's security policy allows no inline styles).
    let class = format!("avatar size-{size}");
    match who.avatar {
        Some(url) => view! { <img class=class src=url alt="" width=size height=size/> }.into_any(),
        None => {
            view! { <span class=format!("{class} initial") aria-hidden="true">{crate::fmt::initial(&who.name)}</span> }
                .into_any()
        }
    }
}

pub fn dot_class(d: Dot) -> &'static str {
    match d {
        Dot::Away => "dot away",
        Dot::InCall => "dot in-call",
        Dot::Listening => "dot listening",
        Dot::Speaking => "dot speaking",
    }
}

/// A level sparkline as SVG bars: the newest `bars` frames (loudness in dBFS, voiced frames highlighted).
#[component]
pub fn Sparkline(levels: Signal<Levels>, #[prop(default = 96)] bars: usize) -> impl IntoView {
    let path = move || {
        let l = levels.get();
        let frames: Vec<_> = l.runs.iter().flat_map(|r| r.frames.iter().copied()).collect();
        let tail = &frames[frames.len().saturating_sub(bars)..];
        let mut quiet = String::new();
        let mut voiced = String::new();
        let offset = bars - tail.len();
        for (i, f) in tail.iter().enumerate() {
            // −60 dBFS and below is flat; 0 dBFS is full height (24).
            let h = ((f.dbfs() + 60.0) / 60.0).clamp(0.02, 1.0) * 24.0;
            let x = offset + i;
            let seg = format!("M{x} {:.1}V24", 24.0 - h);
            if f.probability() >= 0.5 {
                voiced.push_str(&seg)
            } else {
                quiet.push_str(&seg)
            }
        }
        (quiet, voiced)
    };
    view! {
        <svg class="spark" viewBox=format!("0 0 {bars} 24") preserveAspectRatio="none" aria-hidden="true">
            <path class="quiet" d=move || path().0/>
            <path class="voiced" d=move || path().1/>
        </svg>
    }
}

/// A key for `<For>` that changes whenever anything shown in a row changes (a keyed list keeps a row's view while
/// its key stays the same; for small rows drawn once, the whole content is the key).
pub fn content_key<T: serde::Serialize>(item: &T) -> String {
    serde_json::to_string(item).unwrap_or_default()
}
