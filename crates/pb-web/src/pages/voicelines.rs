//! Voice lines: the editor for one scope (what the bot says, as clips and as text per language), and `/voice-lines`
//! with the clip library (upload any audio file, record in the browser, language tag, transcript, self-check) and the
//! global lines.

use leptos::prelude::*;
use pb_domain::{ActionKind, Label, Scope};
use pb_i18n::{Locale, text};
use pb_store_api::ClipRow;
use pb_voicelines::{Line, LineKey, Sel, Slot};

use super::settings::{language_list, scope_param};
use crate::app::{Viewer, app, viewer};
use crate::fmt;
use crate::islands::{ClipRecorder, PreviewPlayer};

/// A line's name.
pub fn line_title(loc: Locale, line: &Line) -> String {
    let t = |id: &str| text(loc, id, &[]);
    match line {
        Line::Warning { label, step } => {
            let what = match label {
                Sel::Any => t("ui-vl-any-type"),
                Sel::Is(l) => fmt::label(loc, *l),
            };
            let when = match step {
                Sel::Any => t("ui-vl-any-step"),
                Sel::Is(n) => text(loc, "ui-vl-step", &[("n", (*n).into())]),
            };
            format!("{} · {what} · {when}", t("ui-vl-warning"))
        }
        Line::Greeting => t("ui-vl-greeting"),
        Line::StrikeNotice => t("ui-vl-strike"),
        Line::Action { kind } => match kind {
            Sel::Any => t("ui-vl-action-any"),
            Sel::Is(k) => text(
                loc,
                "ui-vl-action",
                &[("action", pb_i18n::action_text(loc, *k, None).into())],
            ),
        },
        Line::Say { preset } => text(loc, "ui-vl-say", &[("name", preset.clone().into())]),
        Line::Name => t("ui-vl-name"),
    }
}

/// The lines the editor always shows, then the ones set somewhere that are not among them.
fn lines(scope: Scope) -> Vec<LineKey> {
    let mut out = vec![LineKey(Line::Warning {
        label: Sel::Any,
        step: Sel::Any,
    })];
    out.extend(Label::ALL.iter().map(|l| {
        LineKey(Line::Warning {
            label: Sel::Is(*l),
            step: Sel::Any,
        })
    }));
    out.push(LineKey(Line::Greeting));
    out.push(LineKey(Line::StrikeNotice));
    out.push(LineKey(Line::Action { kind: Sel::Any }));
    for k in [
        ActionKind::Mute,
        ActionKind::Unmute,
        ActionKind::Disconnect,
        ActionKind::Timeout,
    ] {
        out.push(LineKey(Line::Action { kind: Sel::Is(k) }));
    }
    if matches!(scope, Scope::Person { .. }) {
        out.push(LineKey(Line::Name));
    }
    let tree = app().engine.settings().current();
    for s in scopes_above(scope).into_iter().chain([scope]) {
        if let Some(slots) = tree.voice_lines(s) {
            for k in slots.keys() {
                if !out.contains(k) && (!matches!(k.0, Line::Name) || matches!(scope, Scope::Person { .. })) {
                    out.push(k.clone());
                }
            }
        }
    }
    out
}

/// The scopes a scope inherits from, nearest first.
fn scopes_above(scope: Scope) -> Vec<Scope> {
    match scope {
        Scope::Global => vec![],
        Scope::Server { .. } => vec![Scope::Global],
        Scope::Person { guild, .. } => vec![Scope::Server { guild }, Scope::Global],
    }
}

fn scope_name(loc: Locale, s: Scope) -> String {
    text(
        loc,
        match s {
            Scope::Global => "source-global",
            Scope::Server { .. } => "source-server",
            Scope::Person { .. } => "source-person",
        },
        &[],
    )
}

/// The language a new text is most likely in: the scope's fixed voice language, else its first fallback.
fn preferred_lang(scope: Scope) -> String {
    let (g, u) = match scope {
        Scope::Global => (None, None),
        Scope::Server { guild } => (Some(guild), None),
        Scope::Person { guild, user } => (Some(guild), Some(user)),
    };
    let eff = app().engine.settings().current().effective(g, u);
    match &eff.voice_language.value {
        pb_settings::VoiceLang::Fixed(l) => l.to_string(),
        pb_settings::VoiceLang::Auto => eff
            .fallback_languages
            .value
            .first()
            .map(ToString::to_string)
            .unwrap_or_default(),
    }
}

#[component]
fn LangSelect(preferred: String) -> impl IntoView {
    view! {
        <select name="lang">
            {language_list().into_iter().map(|l| {
                let sel = l == preferred;
                view! { <option value=l.clone() selected=sel>{fmt::language(&l)}</option> }
            }).collect_view()}
        </select>
    }
}

/// Hidden fields every voice-line form carries.
#[component]
fn LineFields(v: Viewer, scope: Scope, line: String, back: String) -> impl IntoView {
    view! {
        <input type="hidden" name="csrf" value=v.csrf/>
        <input type="hidden" name="scope" value=scope_param(scope)/>
        <input type="hidden" name="line" value=line.clone()/>
        <input type="hidden" name="back" value=format!("{back}#line-{line}")/>
    }
}

#[component]
fn LineRow(key: LineKey, scope: Scope, v: Viewer, back: String, clips: Vec<ClipRow>) -> impl IntoView {
    let loc = v.locale;
    let tree = app().engine.settings().current();
    let here: Option<Slot> = tree.voice_lines(scope).and_then(|s| s.get(&key)).cloned();
    let inherited = scopes_above(scope).into_iter().find(|s| {
        tree.voice_lines(*s)
            .and_then(|x| x.get(&key))
            .is_some_and(|x| !x.is_empty())
    });
    let badge = match (&here, inherited) {
        (Some(_), _) => text(loc, "ui-set-here", &[]),
        (None, Some(s)) => text(loc, "ui-inherited", &[("from", scope_name(loc, s).into())]),
        (None, None) => text(loc, "ui-built-in", &[]),
    };
    let line = key.to_string();
    let preview = format!("/media/preview?scope={}&line={line}", scope_param(scope));
    let preview_langs = preview_languages(scope, &key, &clips);
    let preferred = preferred_lang(scope);
    let clip_name = |h: &pb_domain::BlobHash| {
        clips
            .iter()
            .find(|c| c.record.render == *h)
            .map_or_else(|| text(loc, "ui-clip-removed", &[]), |c| c.record.name.clone())
    };
    let slot = here.clone().unwrap_or_default();
    view! {
        <div class="vline" id=format!("line-{line}") class:here=here.is_some()>
            <div class="vline-head">
                <b>{line_title(loc, &key.0)}</b>
                <span class="badge" class:here=here.is_some()>{badge}</span>
                <PreviewPlayer src=preview langs=preview_langs locale=loc/>
            </div>
            {(!slot.clips.is_empty()).then(|| view! {
                <ul class="slot-clips">
                    {slot.clips.iter().map(|h| view! {
                        <li>
                            <span>{clip_name(h)}</span>
                            <audio controls preload="none" src=format!("/media/clip/{h}")></audio>
                            <form method="post" action="/voice-lines" class="inline">
                                <LineFields v=v.clone() scope line=line.clone() back=back.clone()/>
                                <input type="hidden" name="op" value="remove_clip"/>
                                <input type="hidden" name="clip" value=h.to_string()/>
                                <button class="link danger">{text(loc, "ui-remove", &[])}</button>
                            </form>
                        </li>
                    }).collect_view()}
                </ul>
            })}
            {slot.text.iter().map(|(l, said)| view! {
                <form method="post" action="/voice-lines" class="row slot-text">
                    <LineFields v=v.clone() scope line=line.clone() back=back.clone()/>
                    <input type="hidden" name="lang" value=l.to_string()/>
                    <span class="lang">{fmt::language(&l.to_string())}</span>
                    <input name="text" class="grow" value=said.clone()/>
                    <button class="button" name="op" value="set_text">{text(loc, "ui-save", &[])}</button>
                    <button class="link danger" name="op" value="remove_text">{text(loc, "ui-remove", &[])}</button>
                </form>
            }).collect_view()}
            <div class="row adders">
                <form method="post" action="/voice-lines" class="row">
                    <LineFields v=v.clone() scope line=line.clone() back=back.clone()/>
                    <input type="hidden" name="op" value="set_text"/>
                    <LangSelect preferred=preferred.clone()/>
                    <input name="text" required placeholder=text(loc, "ui-vl-text-placeholder", &[])/>
                    <button class="button">{text(loc, "ui-add-text", &[])}</button>
                </form>
                {(!clips.is_empty()).then(|| view! {
                    <form method="post" action="/voice-lines" class="row">
                        <LineFields v=v.clone() scope line=line.clone() back=back.clone()/>
                        <input type="hidden" name="op" value="add_clip"/>
                        <select name="clip">
                            {clips.iter().map(|c| {
                                let tag = c.record.lang.as_ref().map(|l| format!(" ({l})")).unwrap_or_default();
                                view! { <option value=c.record.render.to_string()>{format!("{}{tag}", c.record.name)}</option> }
                            }).collect_view()}
                        </select>
                        <button class="button">{text(loc, "ui-add-clip", &[])}</button>
                    </form>
                })}
                {here.is_some().then(|| view! {
                    <form method="post" action="/voice-lines" class="inline">
                        <LineFields v=v.clone() scope line=line.clone() back=back.clone()/>
                        <input type="hidden" name="op" value="clear"/>
                        <button class="link danger">{text(loc, "ui-use-inherited", &[])}</button>
                    </form>
                })}
            </div>
        </div>
    }
}

/// Where a clip is used: every voice line (with the scope it is set at) that has it among its clips.
pub fn clip_uses(tree: &pb_settings::SettingsTree, clip: &pb_domain::BlobHash) -> Vec<(Scope, LineKey)> {
    let mut out = Vec::new();
    let mut scan = |scope: Scope, slots: &pb_voicelines::Slots| {
        out.extend(
            slots
                .iter()
                .filter(|(_, slot)| slot.clips.contains(clip))
                .map(|(k, _)| (scope, k.clone())),
        );
    };
    scan(Scope::Global, &tree.global.voice_lines);
    for (g, server) in &tree.servers {
        scan(Scope::Server { guild: *g }, &server.voice_lines);
        for (u, person) in &server.people {
            scan(Scope::Person { guild: *g, user: *u }, &person.voice_lines);
        }
    }
    out
}

/// The languages a line's preview offers: those of its texts and clips here and above, and those a voice speaks.
fn preview_languages(scope: Scope, key: &LineKey, clips: &[ClipRow]) -> Vec<(String, String)> {
    let tree = app().engine.settings().current();
    let mut codes: Vec<String> = app()
        .engine
        .voices()
        .iter()
        .map(|v| {
            v.language
                .split(['_', '-'])
                .next()
                .unwrap_or(&v.language)
                .to_lowercase()
        })
        .collect();
    for s in scopes_above(scope).into_iter().chain([scope]) {
        if let Some(slot) = tree.voice_lines(s).and_then(|x| x.get(key)) {
            codes.extend(slot.text.keys().map(ToString::to_string));
            codes.extend(slot.clips.iter().filter_map(|h| {
                clips
                    .iter()
                    .find(|c| c.record.render == *h)
                    .and_then(|c| c.record.lang.as_ref().map(ToString::to_string))
            }));
        }
    }
    let mut out: Vec<(String, String)> = codes
        .into_iter()
        .map(|c| {
            let name = fmt::language(&c);
            (c, name)
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// Every voice line at `scope`.
#[component]
pub fn VoiceLinesEditor(scope: Scope, back: String) -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let index = app().index;
    view! {
        <section class="card voice-lines">
            <h2>{text(loc, "ui-nav-voice-lines", &[])}</h2>
            <p class="muted">{text(loc, "ui-vl-help", &[])}</p>
            <Suspense fallback=|| ()>
                {Suspend::new(async move {
                    let clips = index.clips().await.unwrap_or_default();
                    let rows = lines(scope)
                        .into_iter()
                        .map(|key| view! { <LineRow key scope v=v.clone() back=back.clone() clips=clips.clone()/> })
                        .collect_view();
                    view! {
                        {rows}
                        <form method="post" action="/voice-lines" class="row add-line">
                            <input type="hidden" name="csrf" value=v.csrf.clone()/>
                            <input type="hidden" name="scope" value=scope_param(scope)/>
                            <input type="hidden" name="back" value=back.clone()/>
                            <input type="hidden" name="op" value="set_text"/>
                            <select name="line_kind">
                                <option value="warning">{text(loc, "ui-vl-warning", &[])}</option>
                                <option value="say">{text(loc, "ui-vl-say-new", &[])}</option>
                            </select>
                            <select name="line_label">
                                <option value="any">{text(loc, "ui-vl-any-type", &[])}</option>
                                {Label::ALL.iter().map(|l| view! { <option value=l.key()>{fmt::label(loc, *l)}</option> }).collect_view()}
                            </select>
                            <input name="line_step" type="number" min="1" placeholder=text(loc, "ui-vl-step-placeholder", &[])/>
                            <input name="line_preset" placeholder=text(loc, "ui-vl-preset-placeholder", &[])/>
                            <LangSelect preferred=preferred_lang(scope)/>
                            <input name="text" required placeholder=text(loc, "ui-vl-text-placeholder", &[])/>
                            <button class="button">{text(loc, "ui-vl-add-line", &[])}</button>
                        </form>
                    }
                })}
            </Suspense>
        </section>
    }
    .into_any()
}

/// `/voice-lines`: the clip library and the global voice lines.
#[component]
pub fn VoiceLinesPage() -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let index = app().index;
    let langs = language_list();
    let owner = v.owner;
    view! {
        <header class="page-head"><h1>{text(loc, "ui-nav-voice-lines", &[])}</h1></header>
        <section class="card">
            <h2>{text(loc, "ui-clips", &[])}</h2>
            <form method="post" action="/clips" enctype="multipart/form-data" class="row upload">
                <input type="hidden" name="csrf" value=v.csrf.clone()/>
                <input type="hidden" name="back" value="/voice-lines"/>
                <input type="file" name="file" required accept="audio/*,video/webm,video/mp4,.opus,.ogg,.wav,.mp3,.m4a,.flac,.webm,.caf,.mkv"/>
                <input name="name" placeholder=text(loc, "ui-clip-name", &[])/>
                <select name="lang">
                    <option value="">{text(loc, "ui-clip-no-speech", &[])}</option>
                    {langs.iter().map(|l| view! { <option value=l.clone()>{fmt::language(l)}</option> }).collect_view()}
                </select>
                <button class="button primary">{text(loc, "ui-upload", &[])}</button>
            </form>
            <ClipRecorder csrf=v.csrf.clone() locale=loc/>
            <Suspense fallback=|| ()>
                {Suspend::new(async move {
                    let clips = index.clips().await.unwrap_or_default();
                    if clips.is_empty() {
                        return view! { <p class="muted">{text(loc, "ui-no-clips", &[])}</p> }.into_any();
                    }
                    view! {
                        <table class="rows clips">
                            <tbody>
                                {clips.into_iter().map(|c| clip_row(c, &v, loc)).collect_view()}
                            </tbody>
                        </table>
                    }
                    .into_any()
                })}
            </Suspense>
        </section>
        {owner.then(|| view! { <VoiceLinesEditor scope=Scope::Global back="/voice-lines".to_string()/> })}
    }
    .into_any()
}

fn clip_row(c: ClipRow, v: &Viewer, loc: Locale) -> AnyView {
    let r = c.record;
    let flagged = r.self_check.and_then(|s| {
        Label::ALL
            .iter()
            .map(|l| (*l, s[l.index()]))
            .filter(|(_, x)| *x >= 0.5)
            .max_by(|a, b| a.1.total_cmp(&b.1))
    });
    let heard = r
        .heard_language
        .filter(|l| *l != pb_domain::ClfLang::Other)
        .map(|l| fmt::language(l.code()));
    let hash = r.render.to_string();
    let lang = r.lang.as_ref().map(ToString::to_string).unwrap_or_default();
    let details = view! {
        <div class="muted small">
            {fmt::ms(loc, u64::from(r.dur_ms))}
            {heard.map(|h| view! { " · " {text(loc, "ui-clip-heard", &[("lang", h.into())])} })}
            {flagged.map(|(l, s)| view! { " · " <span class="chip warn">{text(loc, "ui-clip-sounds-like", &[("label", fmt::label(loc, l).into()), ("score", fmt::pct(s).into())])}</span> })}
        </div>
    };
    // The library is shared: only who added a clip (and the owner) may change or remove it.
    if !r.editable_by(v.user, v.owner) {
        return view! {
            <tr>
                <td>
                    <b>{r.name.clone()}</b>
                    {(!lang.is_empty()).then(|| view! { " · " {fmt::language(&lang)} })}
                    {r.transcript.clone().map(|t| view! { <div class="small">{t}</div> })}
                    {details}
                </td>
                <td><audio controls preload="none" src=format!("/media/clip/{hash}")></audio></td>
                <td class="right muted small">{r.added_by.name.clone().map(|n| text(loc, "ui-clip-added-by", &[("name", n.into())]))}</td>
            </tr>
        }
        .into_any();
    }
    view! {
        <tr>
            <td>
                <form method="post" action="/clips/update" class="row">
                    <input type="hidden" name="csrf" value=v.csrf.clone()/>
                    <input type="hidden" name="back" value="/voice-lines"/>
                    <input type="hidden" name="clip" value=hash.clone()/>
                    <input name="name" value=r.name.clone()/>
                    <select name="lang">
                        <option value="" selected=lang.is_empty()>{text(loc, "ui-clip-no-speech", &[])}</option>
                        {language_list().into_iter().map(|l| {
                            let sel = l == lang;
                            view! { <option value=l.clone() selected=sel>{fmt::language(&l)}</option> }
                        }).collect_view()}
                    </select>
                    <input name="transcript" class="grow" value=r.transcript.clone().unwrap_or_default() placeholder=text(loc, "ui-clip-transcript", &[])/>
                    <button class="button">{text(loc, "ui-save", &[])}</button>
                </form>
                {details}
            </td>
            <td><audio controls preload="none" src=format!("/media/clip/{hash}")></audio></td>
            <td class="right">
                <form method="post" action="/clips/remove">
                    <input type="hidden" name="csrf" value=v.csrf.clone()/>
                    <input type="hidden" name="back" value="/voice-lines"/>
                    <input type="hidden" name="clip" value=hash.clone()/>
                    <button class="link danger">{text(loc, "ui-remove", &[])}</button>
                </form>
            </td>
        </tr>
    }
    .into_any()
}
