//! The sidebar's communities and people with live dots.

use leptos::prelude::*;
use pb_i18n::{Locale, text};
use pb_live_proto::{SidebarState, Topic, TopicState};

use super::widgets::{content_key, dot_class};

#[island]
pub fn SidebarLive(initial: SidebarState, locale: Locale, current: String) -> impl IntoView {
    let state = RwSignal::new(initial);
    let w = crate::live::watch(Topic::Sidebar, move |s| {
        if let TopicState::Sidebar(s) = s {
            state.set(s.clone());
        }
    });
    on_cleanup(move || w.stop());
    let current = StoredValue::new(current);
    view! {
        <Show when=move || state.with(|s| s.communities.is_empty())>
            <p class="muted small sidebar-hint">{text(locale, "ui-no-communities", &[])}</p>
        </Show>
        <nav class="communities" aria-label=text(locale, "ui-nav-communities", &[])>
            <For each=move || state.get().communities key=content_key let:c>
                {
                    let href = format!("/c/{}", c.id);
                    let active = current.with_value(|cur| cur.starts_with(&href));
                    view! {
                        <div class="community" class:paused=c.paused class:gone=!c.available>
                            <a class="community-name" class:active=active href=href.clone()>{c.name.clone()}</a>
                            <ul class="people">
                                {c.people.iter().map(|p| {
                                    let phref = format!("{href}/p/{}", p.who.user);
                                    let here = current.with_value(|cur| cur.starts_with(&phref));
                                    view! {
                                        <li class:paused=p.paused>
                                            <a href=phref class:active=here>
                                                <span class=dot_class(p.dot)></span>
                                                <span class="name">{p.who.name.clone()}</span>
                                            </a>
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        </div>
                    }
                }
            </For>
        </nav>
    }
}
