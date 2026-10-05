//! The settings of one scope (global, a community, a person), grouped by section, the everyday ones first and the rest
//! folded under "Advanced". Each setting is its own small form: the value, where the value in effect comes from, Save,
//! and "Use inherited" when it is set here; lists of communities, roles and people change one entry at a time. Owner-only
//! settings are shown to admins but not editable. Everything set at the scope can be reset at once (after a
//! confirmation).

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;
use pb_domain::{Label, Scope, ScopeKind};
use pb_i18n::{Locale, choice_id, section_name, setting_help, setting_name, text};
use pb_settings::{FieldKind, Section, SettingKey, SettingsTree, Source, StepAction, Who};
use serde_json::Value;

use crate::app::{Viewer, app, viewer};
use crate::fmt;

/// The scope as the forms send it.
pub fn scope_param(s: Scope) -> String {
    match s {
        Scope::Global => "global".into(),
        Scope::Server { guild } => format!("server:{guild}"),
        Scope::Person { guild, user } => format!("person:{guild}:{user}"),
    }
}

/// The scope a form sends: `global`, `server:<g>`, `person:<g>:<u>` (see [`scope_param`]).
pub fn parse_scope(s: &str) -> Option<Scope> {
    let mut p = s.split(':');
    match (p.next()?, p.next(), p.next()) {
        ("global", None, None) => Some(Scope::Global),
        ("server", Some(g), None) => Some(Scope::Server {
            guild: pb_domain::GuildId(g.parse().ok()?),
        }),
        ("person", Some(g), Some(u)) => Some(Scope::Person {
            guild: pb_domain::GuildId(g.parse().ok()?),
            user: pb_domain::UserId(u.parse().ok()?),
        }),
        _ => None,
    }
}

pub(crate) fn source_id(s: Source) -> &'static str {
    match s {
        Source::Person => "source-person",
        Source::Server => "source-server",
        Source::Global => "source-global",
        Source::File => "source-file",
        Source::Default => "source-builtin",
    }
}

/// The value in effect at `scope` as JSON, and where it comes from.
pub fn effective_value(tree: &SettingsTree, scope: Scope, key: SettingKey) -> (Value, Source) {
    let (g, u) = match scope {
        Scope::Global => (None, None),
        Scope::Server { guild } => (Some(guild), None),
        Scope::Person { guild, user } => (Some(guild), Some(user)),
    };
    let source = tree.source_of(g, u, key);
    let layer = match source {
        Source::Person => g
            .zip(u)
            .map(|(guild, user)| tree.overrides(Scope::Person { guild, user })),
        Source::Server => g.map(|guild| tree.overrides(Scope::Server { guild })),
        Source::Global => Some(tree.overrides(Scope::Global)),
        Source::File => Some(tree.file_defaults.clone()),
        Source::Default => None,
    };
    let value = layer
        .and_then(|l| l.get_json(key))
        .unwrap_or_else(|| key.meta().default);
    (value, source)
}

/// A JSON value as the text an input shows.
fn shown(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(shown).collect::<Vec<_>>().join(", "),
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| format!("{k}={}", shown(v)))
            .collect::<Vec<_>>()
            .join(" "),
        other => other.to_string(),
    }
}

/// A kind of line's name.
fn line_kind_name(loc: Locale, k: pb_voicelines::LineKind) -> String {
    use pb_voicelines::LineKind;
    let id = match k {
        LineKind::Warning => "ui-vl-warning",
        LineKind::StrikeNotice => "ui-vl-strike",
        LineKind::Action => "ui-vl-kind-action",
        LineKind::Greeting => "ui-vl-greeting",
        LineKind::Say => "ui-vl-kind-say",
    };
    text(loc, id, &[])
}

/// The choice list a setting's choices are named by.
/// The languages offered: the classifier's (its language head) and every installed voice's.
pub fn language_list() -> Vec<String> {
    let mut out: Vec<String> = pb_domain::ClfLang::ALL
        .iter()
        .map(|l| l.code().to_owned())
        .filter(|c| c != "other")
        .collect();
    for v in app().engine.voices() {
        let code = v
            .language
            .split(['_', '-'])
            .next()
            .unwrap_or(&v.language)
            .to_lowercase();
        if !out.contains(&code) {
            out.push(code);
        }
    }
    out.sort();
    out
}

#[component]
fn LangOptions(selected: String, #[prop(default = false)] auto: bool, locale: Locale) -> impl IntoView {
    view! {
        {auto.then(|| {
            let sel = selected == "auto";
            view! { <option value="auto" selected=sel>{text(locale, "choice-voice-language-auto", &[])}</option> }
        })}
        {language_list()
            .into_iter()
            .map(|l| {
                let sel = l == selected;
                let name = fmt::language(&l);
                view! { <option value=l.clone() selected=sel>{name}</option> }
            })
            .collect_view()}
    }
}

/// The id of a setting's input (its label points there; the setting's form has the id `set-…`, the anchor saving
/// comes back to).
pub(crate) fn input_id(key: SettingKey) -> String {
    format!("in-{}", key.name())
}

/// The name of a setting's input in a section's form (`value.<key>`).
pub fn input_name(key: SettingKey) -> String {
    format!("value.{}", key.name())
}

/// The input for one setting.
fn input(key: SettingKey, scope: Scope, value: &Value, locale: Locale, disabled: bool) -> AnyView {
    let id = input_id(key);
    let name = input_name(key);
    let s = shown(value);
    match key.meta().kind {
        FieldKind::Bool => {
            let on = value.as_bool().unwrap_or(false);
            view! {
                <select id=id name=name.clone() disabled=disabled>
                    <option value="on" selected=on>{text(locale, "ui-on", &[])}</option>
                    <option value="off" selected=!on>{text(locale, "ui-off", &[])}</option>
                </select>
            }
            .into_any()
        }
        FieldKind::Probability => view! {
            <input id=id name=name.clone() type="number" min="0" max="1" step="0.01" value=s disabled=disabled/>
        }
        .into_any(),
        FieldKind::Count => view! {
            <input id=id name=name.clone() type="number" min="1" step="1" value=s disabled=disabled/>
        }
        .into_any(),
        FieldKind::Rate => view! {
            <input id=id name=name.clone() type="number" min="0.1" step="0.05" value=s disabled=disabled/>
        }
        .into_any(),
        FieldKind::Number { unit } => view! {
            <input id=id name=name.clone() type="number" step="any" value=s disabled=disabled/>
            <span class="unit">{unit}</span>
        }
        .into_any(),
        FieldKind::Duration { unlimited, .. } => {
            let hint = if unlimited {
                text(locale, "ui-duration-or-unlimited", &[])
            } else {
                text(locale, "ui-duration", &[])
            };
            view! { <input id=id name=name.clone() type="text" value=s placeholder=hint.clone() title=hint disabled=disabled/> }
                .into_any()
        }
        FieldKind::Choice { choices } => {
            view! {
                <select id=id name=name.clone() disabled=disabled>
                    {choices
                        .into_iter()
                        .map(|c| {
                            let sel = s == c;
                            view! { <option value=c selected=sel>{text(locale, &pb_i18n::setting_choice_id(key, c), &[])}</option> }
                        })
                        .collect_view()}
                </select>
            }
            .into_any()
        }
        FieldKind::Channel => {
            let guild = scope.guild();
            let channels: Vec<(String, String)> = guild
                .and_then(|g| app().engine.guilds().get(g).cloned())
                .map(|info| {
                    let mut c: Vec<_> = info
                        .channels
                        .values()
                        .filter(|c| c.kind.holds_messages())
                        .map(|c| (c.position, c.id.to_string(), format!("#{}", c.name)))
                        .collect();
                    c.sort();
                    c.into_iter().map(|(_, id, name)| (id, name)).collect()
                })
                .unwrap_or_default();
            view! {
                <select id=id name=name.clone() disabled=disabled>
                    <option value="" selected=s.is_empty()>{text(locale, "ui-none", &[])}</option>
                    {channels
                        .into_iter()
                        .map(|(cid, name)| {
                            let sel = cid == s;
                            view! { <option value=cid selected=sel>{name}</option> }
                        })
                        .collect_view()}
                </select>
            }
            .into_any()
        }
        FieldKind::TimeOfDay => view! { <input id=id name=name.clone() type="time" value=s disabled=disabled/> }.into_any(),
        FieldKind::Origin => view! {
            <input id=id name=name.clone() type="url" value=s placeholder="https://bot.example.org" disabled=disabled/>
        }
        .into_any(),
        FieldKind::Lang => view! {
            <select id=id name=name.clone() disabled=disabled><LangOptions selected=s locale/></select>
        }
        .into_any(),
        FieldKind::VoiceLang => view! {
            <select id=id name=name.clone() disabled=disabled><LangOptions selected=s auto=true locale/></select>
        }
        .into_any(),
        FieldKind::Voices => {
            let installed = app().engine.voices();
            let current = value.as_object().cloned().unwrap_or_default();
            let mut langs: Vec<String> = installed
                .iter()
                .map(|v| {
                    v.language
                        .split(['_', '-'])
                        .next()
                        .unwrap_or(&v.language)
                        .to_lowercase()
                })
                .collect();
            langs.sort();
            langs.dedup();
            view! {
                <div class="voices">
                    {langs
                        .into_iter()
                        .map(|l| {
                            let chosen = current.get(&l).and_then(Value::as_str).unwrap_or_default().to_owned();
                            let options: Vec<String> = installed
                                .iter()
                                .filter(|v| v.language.to_lowercase().starts_with(&l))
                                .map(|v| v.id.clone())
                                .collect();
                            view! {
                                <label class="inline">
                                    {fmt::language(&l)}
                                    <select name=format!("voice.{l}") disabled=disabled>
                                        <option value="" selected=chosen.is_empty()>{text(locale, "ui-voice-default", &[])}</option>
                                        {options
                                            .into_iter()
                                            .map(|o| {
                                                let sel = o == chosen;
                                                view! { <option value=o.clone() selected=sel>{o.clone()}</option> }
                                            })
                                            .collect_view()}
                                    </select>
                                </label>
                            }
                        })
                        .collect_view()}
                </div>
            }
            .into_any()
        }
        FieldKind::LineVoices => {
            let installed = app().engine.voices();
            let current = value.as_object().cloned().unwrap_or_default();
            view! {
                <div class="voices">
                    {pb_voicelines::LineKind::ALL
                        .into_iter()
                        .map(|k| {
                            let chosen = current.get(k.as_str()).and_then(Value::as_str).unwrap_or_default().to_owned();
                            view! {
                                <label class="inline">
                                    {line_kind_name(locale, k)}
                                    <select name=format!("line-voice.{}", k.as_str()) disabled=disabled>
                                        <option value="" selected=chosen.is_empty()>{text(locale, "ui-voice-by-language", &[])}</option>
                                        {installed
                                            .iter()
                                            .map(|v| {
                                                let sel = v.named(&chosen);
                                                let label = format!("{} · {}", v.id, v.model);
                                                view! { <option value=v.full_id() selected=sel>{label}</option> }
                                            })
                                            .collect_view()}
                                    </select>
                                </label>
                            }
                        })
                        .collect_view()}
                </div>
            }
            .into_any()
        }
        FieldKind::Escalation => escalation(value, locale, disabled),
        FieldKind::Tz => {
            let zones: Vec<String> = jiff::tz::db().available().map(|n| n.as_str().to_owned()).collect();
            view! {
                <input id=id name=name.clone() type="text" value=s list="time-zones" autocomplete="off" disabled=disabled/>
                <datalist id="time-zones">
                    {zones.into_iter().map(|z| view! { <option value=z></option> }).collect_view()}
                </datalist>
            }
            .into_any()
        }
        FieldKind::Words => {
            let lines = value
                .as_array()
                .map(|a| a.iter().map(shown).collect::<Vec<_>>().join("\n"))
                .unwrap_or_default();
            view! { <textarea id=id name=name.clone() rows="6" spellcheck="false" disabled=disabled>{lines}</textarea> }
                .into_any()
        }
        FieldKind::Ids { .. } | FieldKind::Hosts | FieldKind::Langs | FieldKind::Prefix => view! {
            <input id=id name=name.clone() type="text" value=s disabled=disabled/>
        }
        .into_any(),
    }
}

/// The escalation steps as rows (and one empty row to add a step).
fn escalation(value: &Value, locale: Locale, disabled: bool) -> AnyView {
    let mut steps: Vec<Value> = value.as_array().cloned().unwrap_or_default();
    steps.push(Value::Null);
    let t = move |id: &str| text(locale, id, &[]);
    view! {
        <table class="escalation">
            <thead>
                <tr>
                    <th>{t("ui-esc-from")}</th>
                    <th>{t("ui-esc-action")}</th>
                    <th>{t("ui-esc-duration")}</th>
                    <th>{t("ui-esc-owner")}</th>
                </tr>
            </thead>
            <tbody>
                {steps
                    .into_iter()
                    .map(|st| {
                        let from = st.get("from").map(shown).unwrap_or_default();
                        let action = st.get("action").map(shown).unwrap_or_else(|| "none".into());
                        let duration = st.get("duration").map(shown).unwrap_or_default();
                        let owner = st.get("notify_owner").and_then(Value::as_bool).unwrap_or(false);
                        let yes_no = move |name: &'static str, on: bool| {
                            view! {
                                <select name=name disabled=disabled>
                                    <option value="off" selected=!on>{t("ui-off")}</option>
                                    <option value="on" selected=on>{t("ui-on")}</option>
                                </select>
                            }
                        };
                        view! {
                            <tr>
                                <td><input name="esc.from" type="number" min="1" step="1" value=from disabled=disabled/></td>
                                <td>
                                    <select name="esc.action" disabled=disabled>
                                        {StepAction::ALL
                                            .iter()
                                            .map(|a| {
                                                let a = a.as_str();
                                                let sel = a == action;
                                                view! { <option value=a selected=sel>{t(&choice_id("step_action", a))}</option> }
                                            })
                                            .collect_view()}
                                    </select>
                                </td>
                                <td><input name="esc.duration" type="text" value=duration placeholder="5m" disabled=disabled/></td>
                                <td>{yes_no("esc.owner", owner)}</td>
                            </tr>
                        }
                    })
                    .collect_view()}
            </tbody>
        </table>
        <p class="muted small">{t("ui-esc-help")}</p>
    }
    .into_any()
}

/// What "reset every setting here" leaves alone: the pause switches (they have buttons of their own) and how the bot
/// reaches Fluxer and is reached (the System section), so a reset never takes the bot offline or locks people out.
pub fn kept_by_reset(key: SettingKey) -> bool {
    key == SettingKey::Paused || key.meta().section == Section::System
}

/// The settings set at `scope` that "reset every setting here" removes for this login (an admin leaves what only the
/// owner may change).
pub fn resettable(scope: Scope, owner: bool) -> Vec<SettingKey> {
    app()
        .engine
        .settings()
        .current()
        .overrides(scope)
        .keys()
        .into_iter()
        .filter(|k| !kept_by_reset(*k) && (owner || k.who() != Who::Owner))
        .collect()
}

/// Settings most people never change: folded away under "Advanced" (timing of the speech cutter, voices and rates,
/// the bot's own plumbing).
pub fn advanced(key: SettingKey) -> bool {
    use SettingKey as K;
    matches!(
        key,
        K::GuildAllowlist
            | K::JoinSettle
            | K::LeaveGrace
            | K::StrikeWindow
            | K::EndSilence
            | K::MaxSentence
            | K::MinVoiced
            | K::MaxReactionDelay
            | K::LabelThreshold(_)
            | K::VolumeDb
            | K::FallbackLanguages
            | K::TtsVoices
            | K::LineVoices
            | K::SpeechRate
            | K::NoSpeakPolicy
            | K::CommandPrefix
            | K::Instance
            | K::AllowedHosts
            | K::CpuThreads
            | K::TtsThreads
    )
}

/// Where a setting's value comes from, in words, with the value it would have otherwise; when it is changed here, a
/// button of the section's form takes it back (the form's other changes are saved with it).
fn inherit_line(key: SettingKey, scope: Scope, is_here: bool, disabled: bool, viewer: &Viewer) -> AnyView {
    let loc = viewer.locale;
    let tree = app().engine.settings().current();
    let gs = app().engine.guilds();
    let shown = |v: &Value| value_label(key, scope.guild(), v, loc);
    // The value one scope up: the settings for every community, the community's, or (globally) the default.
    let (above, from_file) = match scope {
        Scope::Global => match tree.file_defaults.get_json(key) {
            Some(v) => (v, true),
            None => (key.meta().default, false),
        },
        Scope::Server { .. } => (effective_value(&tree, Scope::Global, key).0, false),
        Scope::Person { guild, .. } => (effective_value(&tree, Scope::Server { guild }, key).0, false),
    };
    let (line, back) = match (scope, is_here) {
        (Scope::Global, false) => (
            text(loc, if from_file { "ui-from-file" } else { "ui-default" }, &[]),
            None,
        ),
        (Scope::Server { .. }, false) => (text(loc, "ui-same-as-global", &[]), None),
        (Scope::Person { guild, .. }, false) => (
            text(
                loc,
                "ui-same-as-community",
                &[("community", gs.guild_name(guild).into())],
            ),
            None,
        ),
        (Scope::Global, true) => (
            text(
                loc,
                if from_file {
                    "ui-changed-file"
                } else {
                    "ui-changed-default"
                },
                &[("value", shown(&above).into())],
            ),
            Some(text(loc, "ui-reset-to-default", &[])),
        ),
        (Scope::Server { guild }, true) => (
            text(
                loc,
                "ui-changed-community",
                &[
                    ("community", gs.guild_name(guild).into()),
                    ("value", shown(&above).into()),
                ],
            ),
            Some(text(loc, "ui-use-global-value", &[])),
        ),
        (Scope::Person { guild, user }, true) => (
            text(
                loc,
                "ui-changed-person",
                &[
                    ("person", gs.name(guild, user).into()),
                    ("community", gs.guild_name(guild).into()),
                    ("value", shown(&above).into()),
                ],
            ),
            Some(text(
                loc,
                "ui-use-community-value",
                &[("community", gs.guild_name(guild).into())],
            )),
        ),
    };
    // Lists change one entry at a time: their way back is "Use inherited" in the list itself.
    let back = back.filter(|_| !disabled && !matches!(key.meta().kind, FieldKind::Ids { .. }));
    view! {
        <p class="muted small origin" class:changed=is_here>
            {line}
            {back.map(|label| view! { " · " <button class="link" name="clear" value=key.name()>{label}</button> })}
        </p>
    }
    .into_any()
}

/// Globally: the communities this login sees that have their own value, with it, linked to it there.
fn changed_in(key: SettingKey, viewer: &Viewer) -> Option<AnyView> {
    let loc = viewer.locale;
    let tree = app().engine.settings().current();
    let gs = app().engine.guilds();
    let own: Vec<(pb_domain::GuildId, Value)> = tree
        .servers
        .keys()
        .filter(|g| viewer.may_see(**g))
        .filter_map(|g| {
            tree.overrides(Scope::Server { guild: *g })
                .get_json(key)
                .map(|v| (*g, v))
        })
        .collect();
    if own.is_empty() {
        return None;
    }
    let section = key.meta().section;
    Some(
        view! {
            <p class="muted small changed-in">
                {text(loc, "ui-changed-in", &[("count", own.len().into())])} " "
                {own.into_iter().enumerate().map(|(i, (g, v))| {
                    let href = format!("{}#set-{}", section_href(Scope::Server { guild: g }, section), key.name());
                    let value = value_label(key, Some(g), &v, loc);
                    view! { {(i > 0).then_some(", ")} <a href=href>{gs.guild_name(g)}</a> " (" {value} ")" }
                }).collect_view()}
            </p>
        }
        .into_any(),
    )
}

/// Why a setting does nothing at the moment (a text id), from the settings in effect at its scope: it is still shown,
/// with the reason, so the switch that brings it to life is easy to find.
fn not_relevant(key: SettingKey, scope: Scope) -> Option<&'static str> {
    use SettingKey as K;
    let (g, u) = match scope {
        Scope::Global => (None, None),
        Scope::Server { guild } => (Some(guild), None),
        Scope::Person { guild, user } => (Some(guild), Some(user)),
    };
    let eff = app().engine.settings().current().effective(g, u);
    match key {
        K::StrikeWindow | K::StrikeNotice if eff.strikes.value.get() <= 1 => Some("ui-nr-strikes"),
        K::DigestTime if eff.digest.value == pb_settings::Digest::Off => Some("ui-nr-digest"),
        K::DigestWeekday if eff.digest.value != pb_settings::Digest::Weekly => Some("ui-nr-digest-weekly"),
        K::AnnounceActions if !eff.actions_enabled.value => Some("ui-nr-actions"),
        K::ModlogAudio if g.is_some() && eff.modlog_channel.value.is_none() => Some("ui-nr-modlog"),
        _ => None,
    }
}

/// Settings that only work together with another one: (this one, the other one). Where the other one cannot be set
/// at the scope shown, the row says where it lives, with its value there and a link to it.
const WORKS_WITH: &[(SettingKey, SettingKey)] = &[
    // Audio in the mod log needs the mod-log channel, which is chosen per community.
    (SettingKey::ModlogAudio, SettingKey::ModlogChannel),
];

/// A value as a short text for people (a channel by its name, nothing as "not set").
pub(crate) fn value_label(key: SettingKey, guild: Option<pb_domain::GuildId>, value: &Value, loc: Locale) -> String {
    match (key.meta().kind, value) {
        (_, Value::Null) => text(loc, "ui-not-set", &[]),
        (_, Value::String(s)) if s.is_empty() => text(loc, "ui-not-set", &[]),
        (_, Value::Bool(on)) => text(loc, if *on { "ui-on" } else { "ui-off" }, &[]),
        (FieldKind::Escalation, Value::Array(steps)) => text(loc, "ui-steps", &[("count", steps.len().into())]),
        (FieldKind::Channel, v) => match (guild, shown(v).parse::<u64>()) {
            (Some(g), Ok(c)) => format!("#{}", app().engine.guilds().channel_name(g, pb_domain::ChannelId(c))),
            _ => shown(v),
        },
        (_, v) => shown(v),
    }
}

/// Where `other` (which `key` needs, and which cannot be set at `scope`) lives: per community (with its value in
/// each community this login sees, linked to it there), or globally on the System page.
fn works_with(other: SettingKey, scope: Scope, viewer: &Viewer) -> AnyView {
    let loc = viewer.locale;
    let tree = app().engine.settings().current();
    let anchor = format!("set-{}", other.name());
    let kinds = other.scopes();
    let (place, links): (&str, Vec<(String, String, String)>) = if kinds.contains(&ScopeKind::Server) {
        // The communities it applies to: the scope's own, or every community the bot is in that this login sees.
        let gs = app().engine.guilds();
        let mut guilds: Vec<pb_domain::GuildId> = match scope.guild() {
            Some(g) => vec![g],
            None => gs.guilds.keys().copied().filter(|g| viewer.may_see(*g)).collect(),
        };
        guilds.sort_by_key(|g| gs.guild_name(*g).to_lowercase());
        let links = guilds
            .into_iter()
            .map(|g| {
                let (value, _) = effective_value(&tree, Scope::Server { guild: g }, other);
                (
                    format!(
                        "{}#{anchor}",
                        section_href(Scope::Server { guild: g }, other.meta().section)
                    ),
                    gs.guild_name(g),
                    value_label(other, Some(g), &value, loc),
                )
            })
            .collect();
        ("ui-where-per-community", links)
    } else if kinds.contains(&ScopeKind::Global) {
        let (value, _) = effective_value(&tree, Scope::Global, other);
        let links = viewer
            .owner
            .then(|| {
                (
                    format!("{}#{anchor}", section_href(Scope::Global, other.meta().section)),
                    text(loc, "ui-nav-settings", &[]),
                    value_label(other, None, &value, loc),
                )
            })
            .into_iter()
            .collect();
        ("ui-where-globally", links)
    } else {
        ("ui-where-per-person", Vec::new())
    };
    view! {
        <div class="works-with small">
            <span class="muted">
                {text(
                    loc,
                    "ui-works-with",
                    &[("setting", setting_name(loc, other).into()), ("where", text(loc, place, &[]).into())],
                )}
            </span>
            {(!links.is_empty()).then(|| view! {
                <ul>
                    {links.into_iter().map(|(href, name, value)| view! {
                        <li><a href=href>{name}</a> ": " {value}</li>
                    }).collect_view()}
                </ul>
            })}
        </div>
    }
    .into_any()
}

/// One setting: its name (the help folded behind a "?"), where its value comes from, the input, Save, "Use inherited"
/// when it is set here, and where a setting it works together with lives when that is set elsewhere. A list of
/// communities, roles or people shows its entries instead, each removed on its own, and adds one at a time.
#[component]
fn SettingRow(key: SettingKey, scope: Scope, viewer: Viewer, back: String) -> impl IntoView {
    let loc = viewer.locale;
    let tree = app().engine.settings().current();
    let meta = key.meta();
    let here = tree.overrides(scope).get_json(key);
    let (value, _) = effective_value(&tree, scope, key);
    // What the last save said about this setting; a refused value is shown again as typed.
    let note = crate::app::field_note(&key.name());
    let value = match note.as_ref().and_then(|n| n.typed.clone()) {
        Some(typed) => Value::String(typed),
        None => value,
    };
    let disabled = meta.who == Who::Owner && !viewer.owner;
    let is_here = here.is_some();
    let origin = inherit_line(key, scope, is_here, disabled, &viewer);
    let elsewhere = (scope == Scope::Global && key.scopes().contains(&pb_domain::ScopeKind::Server))
        .then(|| changed_in(key, &viewer))
        .flatten();
    let partner = WORKS_WITH
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, other)| *other)
        .filter(|other| !other.scopes().contains(&scope.kind()))
        .map(|other| works_with(other, scope, &viewer));
    let anchor = format!("set-{}", key.name());
    let head = view! {
        <div class="setting-head">
            <label for=input_id(key)>{setting_name(loc, key)}</label>
            <details class="help">
                <summary title=text(loc, "ui-help", &[])>"?"</summary>
                <p>{setting_help(loc, key)}</p>
            </details>
            // Only who cannot change it needs to be told why.
            {disabled.then(|| view! { <span class="badge owner">{text(loc, "ui-owner-only", &[])}</span> })}
            {super::commands::setting_commands(key, scope, loc)}
        </div>
    };
    if matches!(meta.kind, FieldKind::Ids { .. }) {
        return view! {
            <div class="setting list-setting" id=anchor.clone() class:here=is_here>
                {head}
                <super::lists::ListBody key scope value viewer back=format!("{back}#{anchor}") disabled is_here/>
                {origin}
                {partner}
            </div>
        }
        .into_any();
    }
    view! {
        <div class="setting" id=anchor.clone() class:here=is_here>
            {head}
            <div class="row">
                {input(key, scope, &value, loc, disabled)}
            </div>
            {origin}
            {elsewhere}
            {note.map(|n| view! {
                <p class=if n.ok { "field-note ok" } else { "field-note error" } role=if n.ok { "status" } else { "alert" }>{n.text}</p>
            })}
            {not_relevant(key, scope).map(|id| view! { <p class="muted small not-relevant">{text(loc, id, &[])}</p> })}
            {partner}
        </div>
    }
    .into_any()
}

/// The keys of one section in display order: the general ones, then per detection type (switch, then bar).
fn ordered(keys: Vec<SettingKey>) -> Vec<SettingKey> {
    let (general, mut per_label): (Vec<_>, Vec<_>) = keys
        .into_iter()
        .partition(|k| !matches!(k, SettingKey::LabelEnabled(_) | SettingKey::LabelThreshold(_)));
    per_label.sort_by_key(|k| match k {
        SettingKey::LabelEnabled(l) => (Label::ALL.iter().position(|x| x == l), 0),
        SettingKey::LabelThreshold(l) => (Label::ALL.iter().position(|x| x == l), 1),
        _ => (None, 2),
    });
    general.into_iter().chain(per_label).collect()
}

/// The settings of `section` that can be set at `scope`, in display order.
fn section_keys(scope: Scope, section: Section) -> Vec<SettingKey> {
    let kind = scope.kind();
    ordered(
        SettingKey::all()
            .into_iter()
            .filter(|k| k.scopes().contains(&kind) && k.meta().section == section)
            .collect(),
    )
}

/// The sections a scope's settings pages show (globally the System section is on the System page).
pub fn page_sections(scope: Scope) -> Vec<Section> {
    Section::ALL
        .into_iter()
        .filter(|s| !(scope == Scope::Global && *s == Section::System))
        .filter(|s| !section_keys(scope, *s).is_empty())
        .collect()
}

/// A section by its name in URLs.
pub fn section_of(key: &str) -> Option<Section> {
    Section::ALL.into_iter().find(|s| s.key() == key)
}

/// Where a section of a scope's settings is: its own page (`/settings/detection`, `/c/1/settings/detection`), the
/// System page, or (for a person, whose settings are one page) its place on that page.
pub fn section_href(scope: Scope, section: Section) -> String {
    match (scope, section) {
        (Scope::Global, Section::System) => "/system".into(),
        (Scope::Global, s) => format!("/settings/{}", s.key()),
        (Scope::Server { guild }, s) => format!("/c/{guild}/settings/{}", s.key()),
        (Scope::Person { guild, user }, s) => format!("/c/{guild}/p/{user}/settings#section-{}", s.key()),
    }
}

/// One section's settings: the everyday ones, then the rarely changed ones folded under "Advanced" (open when one of
/// them was just saved). `back`: the page the forms come back to.
#[component]
pub fn SectionCard(scope: Scope, section: Section, back: String) -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    // Lists change one entry at a time with forms of their own: they come after the section's form, never folded.
    let (lists, keys): (Vec<SettingKey>, Vec<SettingKey>) = section_keys(scope, section)
        .into_iter()
        .partition(|k| matches!(k.meta().kind, FieldKind::Ids { .. }));
    let (more, basic): (Vec<SettingKey>, Vec<SettingKey>) = keys.into_iter().partition(|k| advanced(*k));
    // What the form saves: what this login may change (an admin sees the owner's settings, not their inputs).
    let saved: Vec<String> = basic
        .iter()
        .chain(&more)
        .filter(|k| v.owner || k.who() != Who::Owner)
        .map(|k| k.name())
        .collect();
    let open = use_query_map().with_untracked(|q| q.get("advanced").as_deref() == Some(section.key()));
    let row = |key: SettingKey| view! { <SettingRow key scope viewer=v.clone() back=back.clone()/> };
    // The chat commands' reference beside their settings (globally).
    let commands = (section == Section::Commands && scope == Scope::Global)
        .then(|| view! { <super::commands::ChatCommands guild=None locale=loc/> });
    let folded = (!more.is_empty()).then(|| {
        view! {
            <details class="advanced" id=format!("advanced-{}", section.key()) open=open>
                <summary><h3>{text(loc, "ui-advanced", &[])}</h3></summary>
                <p class="muted small">{text(loc, "ui-advanced-help", &[])}</p>
                {more.into_iter().map(row).collect_view()}
            </details>
        }
    });
    let save = (!saved.is_empty()).then(|| {
        view! {
            <div class="row section-save">
                <button class="button primary" name="action" value="save">{text(loc, "ui-save-changes", &[])}</button>
            </div>
        }
    });
    view! {
        <section class="card settings-section" id=format!("section-{}", section.key())>
            <h2>{section_name(loc, section)}</h2>
            // One form, one "Save changes": only what was changed in it is stored.
            <form method="post" action="/settings" class="section-form">
                <input type="hidden" name="csrf" value=v.csrf.clone()/>
                <input type="hidden" name="scope" value=scope_param(scope)/>
                <input type="hidden" name="back" value=back.clone()/>
                <input type="hidden" name="keys" value=saved.join(",")/>
                {basic.into_iter().map(row).collect_view()}
                {folded}
                {save}
            </form>
            {lists.into_iter().map(row).collect_view()}
            {commands}
        </section>
    }
    .into_any()
}

/// The links to a scope's settings sections (the current one marked).
#[component]
fn SectionNav(scope: Scope, current: Option<Section>) -> impl IntoView {
    let loc = crate::app::locale();
    view! {
        <nav class="settings-menu">
            {page_sections(scope)
                .into_iter()
                .map(|s| view! { <a href=section_href(scope, s) class:active=current == Some(s)>{section_name(loc, s)}</a> })
                .collect_view()}
        </nav>
    }
}

/// Resetting everything set at `scope` (after a confirmation), when anything is.
#[component]
fn ResetCard(scope: Scope, back: String) -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let resettable = resettable(scope, v.owner).len();
    (resettable > 0)
        .then(|| {
            view! {
                <section class="card settings-reset">
                    <form method="post" action="/settings/reset" class="row">
                        <input type="hidden" name="csrf" value=v.csrf.clone()/>
                        <input type="hidden" name="scope" value=scope_param(scope)/>
                        <input type="hidden" name="back" value=back/>
                        <span class="grow muted">{text(loc, "ui-reset-settings-help", &[("count", resettable.into())])}</span>
                        <button class="button danger">{text(loc, "ui-reset-settings", &[])}</button>
                    </form>
                </section>
            }
        })
        .into_any()
}

/// A scope's settings: one section on its own page, or (a person's, `section` `None`) every section on one page.
#[component]
pub fn SettingsForm(scope: Scope, back: String, #[prop(optional)] section: Option<Section>) -> impl IntoView {
    let sections = match section {
        Some(s) => vec![s],
        None => page_sections(scope),
    };
    let nav = match (scope, section) {
        // A person's sections are on one page: the menu jumps.
        (Scope::Person { .. }, _) | (_, None) => view! {
            <nav class="settings-menu">
                {sections.iter().map(|s| {
                    let loc = crate::app::locale();
                    view! { <a href=format!("#section-{}", s.key())>{section_name(loc, *s)}</a> }
                }).collect_view()}
            </nav>
        }
        .into_any(),
        _ => view! { <SectionNav scope current=section/> }.into_any(),
    };
    let reset = section
        .is_none()
        .then(|| view! { <ResetCard scope back=back.clone()/> });
    let loc = crate::app::locale();
    let gs = app().engine.guilds();
    let intro = match scope {
        Scope::Global => text(loc, "ui-settings-intro-global", &[]),
        Scope::Server { guild } => text(
            loc,
            "ui-settings-intro-server",
            &[("community", gs.guild_name(guild).into())],
        ),
        Scope::Person { guild, user } => text(
            loc,
            "ui-settings-intro-person",
            &[
                ("person", gs.name(guild, user).into()),
                ("community", gs.guild_name(guild).into()),
            ],
        ),
    };
    view! {
        <p class="muted">{intro}</p>
        {nav}
        {sections.into_iter().map(|s| view! { <SectionCard scope section=s back=back.clone()/> }).collect_view()}
        {reset}
    }
}

/// `/settings` and a community's Settings tab: what each section holds and how much is set here, and resetting
/// everything set here.
#[component]
pub fn SettingsHome(scope: Scope, back: String) -> impl IntoView {
    let loc = crate::app::locale();
    let set_here = app().engine.settings().current().overrides(scope).keys();
    let intro = match scope {
        Scope::Global => text(loc, "ui-settings-intro-global", &[]),
        Scope::Server { guild } => text(
            loc,
            "ui-settings-intro-server",
            &[("community", app().engine.guilds().guild_name(guild).into())],
        ),
        Scope::Person { .. } => String::new(),
    };
    let tree = app().engine.settings().current();
    let rows = page_sections(scope)
        .into_iter()
        .map(|s| {
            let mine: Vec<SettingKey> = set_here.iter().copied().filter(|k| k.meta().section == s).collect();
            let n = mine.len();
            // What is changed here, with its value.
            let changes = mine
                .into_iter()
                .filter_map(|k| {
                    let v = tree.overrides(scope).get_json(k)?;
                    let value = value_label(k, scope.guild(), &v, loc);
                    Some(view! { <li>{format!("{}: ", setting_name(loc, k))}<b>{value}</b></li> })
                })
                .collect_view();
            view! {
                <li>
                    <a href=section_href(scope, s)>{section_name(loc, s)}</a>
                    {(n > 0).then(|| view! { <span class="muted small">{text(loc, "ui-settings-set-here", &[("count", n.into())])}</span> })}
                    {(n > 0).then(|| view! { <ul class="changes small">{changes}</ul> })}
                </li>
            }
        })
        .collect_view();
    view! {
        <p class="muted">{intro}</p>
        <section class="card">
            <ul class="section-list">{rows}</ul>
        </section>
        <ResetCard scope back/>
    }
}

/// `/settings` and `/settings/:section` (the bot owner): the settings for every community.
#[component]
pub fn GlobalSettingsPage() -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    if !v.owner {
        return view! { <super::NotFound/> }.into_any();
    }
    let loc = v.locale;
    let param = leptos_router::hooks::use_params_map().with_untracked(|p| p.get("section"));
    let section = match param.as_deref().map(section_of) {
        None => None,
        Some(Some(s)) if page_sections(Scope::Global).contains(&s) => Some(s),
        Some(_) => return view! { <super::NotFound/> }.into_any(),
    };
    let back = match section {
        Some(s) => section_href(Scope::Global, s),
        None => "/settings".to_owned(),
    };
    view! {
        <header class="page-head"><h1>{text(loc, "ui-settings-global-title", &[])}</h1></header>
        {match section {
            Some(s) => view! {
                <p class="muted">{text(loc, "ui-settings-intro-global", &[])}</p>
                <SectionNav scope=Scope::Global current=Some(s)/>
                <SectionCard scope=Scope::Global section=s back/>
            }
            .into_any(),
            None => view! { <SettingsHome scope=Scope::Global back/> }.into_any(),
        }}
    }
    .into_any()
}
