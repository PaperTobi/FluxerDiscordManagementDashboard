//! A community's live overview: who is in which call (and whether the bot listens), the tracked people, the bot's
//! connections and the latest violations.

use leptos::prelude::*;
use pb_i18n::{Locale, text};
use pb_live_proto::{BotJoin, GuildState, Topic, TopicState};

use super::widgets::{Avatar, content_key};
use crate::fmt;

#[island]
pub fn GuildLive(initial: GuildState, locale: Locale, csrf: String, back: String) -> impl IntoView {
    let guild = initial.id;
    let state = RwSignal::new(initial);
    let w = crate::live::watch(Topic::Guild { guild }, move |s| {
        if let TopicState::Guild(s) = s {
            state.set((**s).clone());
        }
    });
    on_cleanup(move || w.stop());
    let now = RwSignal::new(crate::live::server_now());
    #[cfg(feature = "hydrate")]
    {
        let tick = set_interval_with_handle(
            move || now.set(crate::live::server_now()),
            std::time::Duration::from_secs(1),
        )
        .ok();
        on_cleanup(move || {
            if let Some(t) = tick {
                t.clear();
            }
        });
    }
    let t = move |id: &str| text(locale, id, &[]);
    let csrf = StoredValue::new(csrf);
    let back = StoredValue::new(back);
    // Joining paused after repeated removals: say so, with until when, and let an admin end it.
    let paused = move || {
        state.with(|s| s.joins_paused_until_ms).map(|until| {
            let seconds = ((until - now.get()).max(0) + 999) / 1000;
            view! {
                <div class="notice warn">
                    <p>{text(locale, "ui-joins-paused", &[("seconds", seconds.into())])}</p>
                    <form method="post" action="/community/resume-joining" class="inline">
                        <input type="hidden" name="csrf" value=csrf.get_value()/>
                        <input type="hidden" name="guild" value=guild.to_string()/>
                        <input type="hidden" name="back" value=back.get_value()/>
                        <button class="button">{t("ui-joins-resume")}</button>
                    </form>
                </div>
            }
        })
    };
    view! {
        {paused}
        <div class="grid two">
            <section class="card">
                <h2>{t("ui-calls")}</h2>
                <Show when=move || !state.with(|s| s.calls.is_empty()) fallback=move || view! { <p class="muted">{t("ui-no-calls")}</p> }>
                    <For each=move || state.get().calls key=content_key let:call>
                        <div class="call">
                            <h3>
                                {call.channel.name.clone()}
                                {call.bot_in.then(|| view! { <span class="chip ok">{t("ui-bot-listens")}</span> })}
                                {let ch = call.channel.id; move || {
                                    let join = state.with(|s| s.connections.iter().find(|c| c.channel == ch).map(|c| c.state));
                                    let id = match join? {
                                        BotJoin::Waiting | BotJoin::Joining => "ui-bot-joining",
                                        BotJoin::Retrying => "ui-bot-retrying",
                                        BotJoin::Leaving => "ui-bot-leaving",
                                        BotJoin::Connected => return None,
                                    };
                                    Some(view! { <span class="chip warn">{t(id)}</span> })
                                }}
                                {(call.bot_in && !call.can_speak).then(|| view! { <span class="chip warn">{t("ui-bot-cannot-speak")}</span> })}
                                {call.encrypted.then(|| view! { <span class="chip">{t("ui-encrypted")}</span> })}
                            </h3>
                            <ul class="participants">
                                {call.participants.iter().map(|p| {
                                    let href = format!("/c/{guild}/p/{}", p.who.user);
                                    let dot = if p.listening { "dot listening" } else { "dot in-call" };
                                    view! {
                                        <li class:bot=p.bot>
                                            <span class=dot></span>
                                            {if p.bot {
                                                view! { <span class="name">{p.who.name.clone()}</span> }.into_any()
                                            } else {
                                                view! { <a href=href class="name">{p.who.name.clone()}</a> }.into_any()
                                            }}
                                            {p.tracked.then(|| view! { <span class="chip">{t("ui-tracked")}</span> })}
                                            {p.muted.then(|| view! { <span class="muted small">{t("ui-muted")}</span> })}
                                            {p.deaf.then(|| view! { <span class="muted small">{t("ui-deafened")}</span> })}
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        </div>
                    </For>
                </Show>
            </section>
            <section class="card">
                <h2>{t("ui-tracked-people")}</h2>
                <Show when=move || !state.with(|s| s.tracked.is_empty()) fallback=move || view! { <p class="muted">{t("ui-nobody-tracked")}</p> }>
                    <ul class="people-list">
                        // Each row with the name of the call it shows (so the row is redrawn when either changes).
                        <For each=move || state.with(|s| s.tracked.iter().map(|p| {
                                let call = p.channel.and_then(|c| s.calls.iter().find(|x| x.channel.id == c)).map(|x| x.channel.name.clone());
                                (p.clone(), call)
                            }).collect::<Vec<_>>())
                            key=content_key let:row>
                            {
                                let (p, call) = row;
                                let user = p.who.user;
                                let href = format!("/c/{guild}/p/{user}");
                                let where_ = match (&p.channel, call) {
                                    (Some(_), Some(name)) => text(locale, "ui-in-channel", &[("channel", name.into())]),
                                    (Some(_), None) => text(locale, "ui-in-channel", &[("channel", String::new().into())]),
                                    (None, _) => text(locale, "ui-not-in-voice", &[]),
                                };
                                view! {
                                    <li>
                                        <Avatar who=p.who.clone() size=28/>
                                        <div class="grow">
                                            <a href=href class="name">{p.who.name.clone()}</a>
                                            <div class="muted small">{where_}</div>
                                        </div>
                                        {p.paused.then(|| view! { <span class="chip warn">{t("ui-paused")}</span> })}
                                        {p.everywhere.then(|| view! { <span class="chip">{t("ui-everywhere")}</span> })}
                                        {(!p.everywhere).then(|| view! {
                                            <form method="post" action="/people/untrack">
                                                <input type="hidden" name="csrf" value=csrf.get_value()/>
                                                <input type="hidden" name="back" value=back.get_value()/>
                                                <input type="hidden" name="guild" value=guild.to_string()/>
                                                <input type="hidden" name="user" value=user.to_string()/>
                                                <button class="link danger">{t("ui-untrack")}</button>
                                            </form>
                                        })}
                                    </li>
                                }
                            }
                        </For>
                    </ul>
                    {move || state.with(|s| s.tracked.iter().any(|p| p.everywhere)).then(|| view! {
                        <p class="muted small">{t("ui-everywhere-hint")}</p>
                    })}
                </Show>
            </section>
        </div>
        <section class="card">
            <h2>{t("ui-wall-violations")}</h2>
            <Show when=move || !state.with(|s| s.violations.is_empty()) fallback=move || view! { <p class="muted">{t("ui-wall-no-violations")}</p> }>
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
