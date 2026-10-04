//! A person's live view: where they are, their microphone level, the conveyor of their latest sentences (each card at
//! the station its timestamps put it at, so a tab that comes back shows the right picture at once), the counts and
//! what the bot said or did.

use leptos::prelude::*;
use pb_domain::PlayPurpose;
use pb_i18n::{Locale, text};
use pb_live_proto::{Activity, PersonState, SentenceCard, Station, Topic, TopicState, display_stage};

use super::widgets::Sparkline;
use crate::fmt;

const STATIONS: [Station; 6] = [
    Station::Recording,
    Station::Cut,
    Station::Queued,
    Station::Model,
    Station::Verdict,
    Station::Decision,
];

#[island]
pub fn PersonLive(initial: PersonState, locale: Locale) -> impl IntoView {
    let (guild, user) = (initial.guild, initial.who.user);
    let state = RwSignal::new(initial);
    let w = crate::live::watch(Topic::Person { guild, user }, move |s| {
        if let TopicState::Person(s) = s {
            state.set((**s).clone());
        }
    });
    on_cleanup(move || w.stop());
    let now = RwSignal::new(crate::live::server_now());
    #[cfg(feature = "hydrate")]
    {
        let tick = set_interval_with_handle(
            move || now.set(crate::live::server_now()),
            std::time::Duration::from_millis(200),
        )
        .ok();
        on_cleanup(move || {
            if let Some(t) = tick {
                t.clear();
            }
        });
    }
    let t = move |id: &str| text(locale, id, &[]);
    let levels = Signal::derive(move || state.with(|s| s.levels.clone()));
    let presence = move || {
        state.with(|s| match &s.presence.channel {
            Some(c) => {
                let mut out = text(locale, "ui-in-channel", &[("channel", c.name.clone().into())]);
                if s.presence.listening {
                    out.push_str(" · ");
                    out.push_str(&text(locale, "ui-bot-listens", &[]));
                }
                if s.presence.muted {
                    out.push_str(" · ");
                    out.push_str(&text(locale, "ui-muted", &[]));
                }
                if s.presence.deaf {
                    out.push_str(" · ");
                    out.push_str(&text(locale, "ui-deafened", &[]));
                }
                out
            }
            None => text(locale, "ui-not-in-voice", &[]),
        })
    };
    let counts = move || state.with(|s| s.counts.clone());
    // A card is drawn again whenever anything about it changed (new stamps, the verdict, the decision).
    let card_key = |c: &SentenceCard| {
        format!(
            "{}:{:?}:{:?}:{:?}",
            c.id,
            c.stamps,
            c.verdict.as_ref().map(|v| v.scores),
            c.decision
        )
    };
    // Newest first.
    let activity_list = move || state.get().activity.into_iter().rev().collect::<Vec<_>>();
    let sentence_list = move || state.get().sentences.into_iter().rev().collect::<Vec<_>>();
    view! {
        <div class="grid two">
            <section class="card">
                <h2>{t("ui-now")}</h2>
                <p class="presence">{presence}</p>
                <Sparkline levels bars=240/>
                <dl class="facts">
                    <dt>{t("ui-jar")}</dt><dd>{move || counts().jar}</dd>
                    <dt>{t("ui-today")}</dt><dd>{move || counts().today}</dd>
                    <dt>{t("ui-in-window")}</dt>
                    <dd>{move || {
                        let c = counts();
                        let window = fmt::window(locale, c.window_ms);
                        text(locale, "ui-in-window-value", &[("n", c.in_window.into()), ("window", window.into())])
                    }}</dd>
                    // `step` already is the step of the next violation (0: none set for it).
                    <dt>{t("ui-next-step")}</dt><dd>{move || match counts().step {
                        0 => "—".to_owned(),
                        n => n.to_string(),
                    }}</dd>
                </dl>
                {move || state.with(|s| s.summary.observe_only).then(|| view! { <p class="chip warn">{t("ui-observe-only")}</p> })}
            </section>
            <section class="card">
                <h2>{t("ui-said-and-done")}</h2>
                <Show when=move || !state.with(|s| s.activity.is_empty()) fallback=move || view! { <p class="muted">{t("ui-nothing-yet")}</p> }>
                    <ul class="feed">
                        <For each=activity_list key=|a| a.id() let:a>
                            {activity(a, locale, now)}
                        </For>
                    </ul>
                </Show>
            </section>
        </div>
        <section class="card conveyor">
            <h2>{t("ui-conveyor")}</h2>
            <div class="belt-head">
                {STATIONS.iter().map(|s| view! { <span>{fmt::station(locale, *s)}</span> }).collect_view()}
            </div>
            <Show when=move || !state.with(|s| s.sentences.is_empty()) fallback=move || view! { <p class="muted">{t("ui-no-sentences")}</p> }>
                <For each=sentence_list key=card_key let:card>
                    <Belt card now locale/>
                </For>
            </Show>
        </section>
    }
}

/// One sentence on the belt: a marker at its station, then what came out.
#[component]
fn Belt(card: SentenceCard, now: RwSignal<i64>, locale: Locale) -> impl IntoView {
    let stamps = card.stamps;
    let shown = move || display_stage(&stamps, now.get());
    let pos = move || STATIONS.iter().position(|s| *s == shown().station).unwrap_or(0);
    let verdict = card.verdict.clone();
    let decision = card.decision;
    let dur = card.dur_ms.map(|d| fmt::ms(locale, u64::from(d)));
    view! {
        <div class="belt" data-card=card.id.to_string() data-station=move || format!("{:?}", shown().station).to_lowercase()>
            <span class="no">"#" {card.no}</span>
            <div class="track">
                {STATIONS.iter().enumerate().map(|(i, _)| view! {
                    <span class="stop" class:passed=move || i < pos() class:at=move || i == pos()></span>
                }).collect_view()}
            </div>
            <span class="out">
                {dur}
                {card.dropped.map(|d| view! { <span class="chip">{fmt::dropped(locale, d)}</span> })}
                {card.error.clone().map(|e| view! { <span class="chip bad" title=e>{text(locale, "ui-failed", &[])}</span> })}
                {move || (shown().station == Station::Decision).then(|| {
                    let top = verdict.as_ref().and_then(crate::fmt::flagged);
                    view! {
                        {top.map(|(l, sc)| view! { <span class="top">{fmt::label(locale, l)} " " {fmt::pct(sc)}</span> })}
                        {decision.map(|d| view! { <span class=format!("chip {}", fmt::decision_class(d))>{fmt::decision(locale, d)}</span> })}
                    }
                })}
            </span>
        </div>
    }
}

fn activity(a: Activity, locale: Locale, now: RwSignal<i64>) -> impl IntoView {
    match a {
        Activity::Play {
            kind,
            at_ms,
            text: said,
            ok,
            ..
        } => {
            let what = text(
                locale,
                match kind {
                    PlayPurpose::Warning => "ui-play-warning",
                    PlayPurpose::StrikeNotice => "ui-play-strike",
                    PlayPurpose::ActionNotice => "ui-play-action",
                    PlayPurpose::Greeting => "ui-play-greeting",
                    PlayPurpose::Say => "ui-play-say",
                },
                &[],
            );
            view! {
                <li>
                    <b>{what}</b>
                    {said.map(|s| view! { <span class="muted">" “" {s} "”"</span> })}
                    {(ok == Some(false)).then(|| view! { <span class="chip bad">{text(locale, "ui-failed", &[])}</span> })}
                    <span class="when">{move || fmt::ago(locale, now.get(), at_ms)}</span>
                </li>
            }
            .into_any()
        }
        Activity::Action {
            kind,
            at_ms,
            secs,
            result,
            ..
        } => view! {
            <li>
                <b>{pb_i18n::action_text(locale, kind, secs)}</b>
                " " <span class="muted">{pb_i18n::action_outcome(locale, &result)}</span>
                <span class="when">{move || fmt::ago(locale, now.get(), at_ms)}</span>
            </li>
        }
        .into_any(),
    }
}
