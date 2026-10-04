//! "Say now" works only while the person is in a call the bot is in: the forms inside are switched off (with the
//! reason) otherwise, live as they come and go. Without scripts the state is the one the page was loaded with.

use leptos::prelude::*;
use pb_domain::{GuildId, UserId};
use pb_i18n::{Locale, text};
use pb_live_proto::{Presence, Topic, TopicState};

/// Why nothing can be said to them now (a text id), or `None`.
fn why_not(p: &Presence) -> Option<&'static str> {
    match (&p.channel, p.bot_in_call) {
        (None, _) => Some("err-not-in-call"),
        (Some(_), false) => Some("ui-say-bot-not-in-call"),
        (Some(_), true) => None,
    }
}

#[island]
pub fn SayGate(guild: GuildId, user: UserId, initial: Presence, locale: Locale, children: Children) -> impl IntoView {
    let presence = RwSignal::new(initial);
    let w = crate::live::watch(Topic::Person { guild, user }, move |s| {
        if let TopicState::Person(s) = s {
            presence.set(s.presence.clone());
        }
    });
    on_cleanup(move || w.stop());
    let why = move || presence.with(why_not);
    view! {
        <fieldset class="say-gate" disabled=move || why().is_some()>{children()}</fieldset>
        {move || why().map(|id| view! { <p class="muted small say-why">{text(locale, id, &[])}</p> })}
    }
}
