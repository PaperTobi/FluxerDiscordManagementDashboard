//! The bot's status, live: Fluxer, the models, the work queues, storage.

use leptos::prelude::*;
use pb_i18n::{Locale, text};
use pb_live_proto::{FluxerState, Model, ModelProblem, PartState, Queue, SystemState, Topic, TopicState};

use crate::fmt;

#[island]
pub fn SystemLive(initial: SystemState, locale: Locale, csrf: String) -> impl IntoView {
    let state = RwSignal::new(initial);
    let w = crate::live::watch(Topic::System, move |s| {
        if let TopicState::System(s) = s {
            state.set((**s).clone());
        }
    });
    on_cleanup(move || w.stop());
    let t = move |id: &str| text(locale, id, &[]);
    let fluxer = move || {
        state.with(|s| match &s.fluxer {
            FluxerState::NoToken => (t("ui-fluxer-no-token"), "warn"),
            FluxerState::Connecting => (t("ui-fluxer-connecting"), "muted"),
            FluxerState::Ready { bot } => (
                text(locale, "ui-fluxer-ready", &[("bot", bot.name.clone().into())]),
                "ok",
            ),
            FluxerState::Reconnecting { error, .. } => (
                text(locale, "ui-fluxer-reconnecting", &[("error", error.clone().into())]),
                "warn",
            ),
            FluxerState::NoVoice => (t("ui-fluxer-no-voice"), "bad"),
            FluxerState::TokenRejected => (t("ui-fluxer-rejected"), "bad"),
            FluxerState::Stopped { error } => (
                text(locale, "ui-fluxer-stopped", &[("error", error.clone().into())]),
                "bad",
            ),
        })
    };
    view! {
        <section class="card">
            <h2>{t("ui-status")}</h2>
            <dl class="facts">
                <dt>{t("ui-version")}</dt><dd>{move || state.with(|s| s.version.clone())}</dd>
                <dt>"Fluxer"</dt><dd>{move || { let (m, c) = fluxer(); view! { <span class=format!("chip {c}")>{m}</span> } }}</dd>
                <dt>{t("ui-calls")}</dt><dd>{move || state.with(|s| s.rooms)}</dd>
                <dt>{t("ui-microphones")}</dt><dd>{move || state.with(|s| s.streams)}</dd>
            </dl>
            <h3>{t("ui-models")}</h3>
            <table class="rows">
                <tbody>
                    {move || state.with(|s| s.models.clone()).into_iter().map(|m| {
                        let name = match m.model {
                            Model::Classifier => t("ui-model-classifier"),
                            Model::VoiceActivity => t("ui-model-voice-activity"),
                            Model::Speech => t("ui-model-speech"),
                        };
                        let state = match m.problem {
                            None => view! { <span class="chip ok">{t("ui-ready")}</span> }.into_any(),
                            Some(ModelProblem::NotAnswering) => view! { <span class="chip bad">{t("ui-model-not-answering")}</span> }.into_any(),
                            Some(ModelProblem::NoVoices) => view! { <span class="chip warn">{t("ui-model-no-voices")}</span> }.into_any(),
                        };
                        view! {
                            <tr>
                                <td>{name}</td>
                                <td class="muted">{m.device.clone()}</td>
                                <td>{state}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
            <h3>{t("ui-queues")}</h3>
            <table class="rows">
                <thead><tr><th></th><th class="num">{t("ui-waiting")}</th><th class="num">{t("ui-done")}</th><th class="num">{t("ui-oldest")}</th></tr></thead>
                <tbody>
                    {move || state.with(|s| s.queues.clone()).into_iter().map(|q| view! {
                        <tr>
                            <td>{match q.queue {
                                Queue::Scoring => t("ui-queue-scoring"),
                                Queue::Speech => t("ui-queue-speech"),
                            }}</td>
                            <td class="num">{q.waiting}</td>
                            <td class="num">{q.done}</td>
                            <td class="num">{fmt::ms(locale, q.oldest_ms)}</td>
                        </tr>
                    }).collect_view()}
                </tbody>
            </table>
            <h3>{t("ui-parts")}</h3>
            <table class="rows">
                <thead><tr><th></th><th></th><th class="num">{t("ui-restarts")}</th><th class="num">{t("ui-waiting")}</th></tr></thead>
                <tbody>
                    {move || state.with(|s| s.parts.clone()).into_iter().map(|p| {
                        let name = match p.name.as_str() {
                            "moderation" => t("ui-part-moderation"),
                            "undo" => t("ui-part-undo"),
                            "digest" => t("ui-part-digest"),
                            "threads" => t("ui-part-threads"),
                            "views" => t("ui-part-views"),
                            "recorder" => t("ui-part-recorder"),
                            "enforcer" => t("ui-part-enforcer"),
                            "gateway" => t("ui-part-gateway"),
                            "system" => t("ui-part-system"),
                            other => other.to_owned(),
                        };
                        let (label, class) = match p.state {
                            PartState::Running => (t("ui-part-running"), "ok"),
                            PartState::Restarting => (t("ui-part-restarting"), "warn"),
                            PartState::NotAnswering => (t("ui-part-not-answering"), "bad"),
                            PartState::Stopped => (t("ui-part-stopped"), "muted"),
                            PartState::Failed => (t("ui-part-failed"), "bad"),
                        };
                        view! {
                            <tr title=p.error.unwrap_or_default()>
                                <td>{name}</td>
                                <td><span class=format!("chip {class}")>{label}</span></td>
                                <td class="num">{p.restarts}</td>
                                <td class="num">{p.waiting}</td>
                            </tr>
                        }
                    }).collect_view()}
                </tbody>
            </table>
            <h3>{t("ui-storage")}</h3>
            <dl class="facts">
                <dt>{t("ui-log")}</dt><dd>{move || fmt::bytes(state.with(|s| s.storage.log_bytes))}</dd>
                <dt>{t("ui-files")}</dt><dd>{move || fmt::bytes(state.with(|s| s.storage.blob_bytes))}</dd>
                <dt>{t("ui-free")}</dt><dd>{move || fmt::bytes(state.with(|s| s.storage.free_bytes))}</dd>
                <dt>{t("ui-index-behind")}</dt><dd>{move || state.with(|s| s.storage.index_behind)}</dd>
            </dl>
            {move || state.with(|s| s.storage.index_problem.clone()).map(|e| view! {
                <p class="notice error" role="alert">{text(locale, "ui-index-problem", &[("error", e.into())])}</p>
            })}
            {move || {
                let n = state.with(|s| s.storage.index_skipped);
                (n > 0).then(|| view! { <p class="notice">{text(locale, "ui-index-skipped", &[("count", n.into())])}</p> })
            }}
            {move || state.with(|s| s.storage.log_halted.clone()).map(|e| view! {
                <div class="notice error" role="alert">
                    <p>{text(locale, "ui-log-halted", &[("error", e.into())])}</p>
                    <form method="post" action="/system/retry-log">
                        <input type="hidden" name="csrf" value=csrf.clone()/>
                        <input type="hidden" name="back" value="/system"/>
                        <button class="button">{t("ui-log-retry")}</button>
                    </form>
                </div>
            })}
        </section>
    }
}
