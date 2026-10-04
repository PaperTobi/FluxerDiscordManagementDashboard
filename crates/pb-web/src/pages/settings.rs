//! The settings of one scope (global, a community, a person), grouped by section, the everyday ones first and the rest
//! folded under "Advanced". Each setting is its own small form: the value, where the value in effect comes from, Save,
//! and "Use inherited" when it is set here; lists of communities, roles and people are picked by name. Owner-only
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

fn source_id(s: Source) -> &'static str {
    match s {
        Source::Person => "source-person",
        Source::Server => "source-server",
        Source::Global => "source-global",
        Source::File => "source-file",
        Source::Default => "source-builtin",
    }
}

/// The value in effect at `scope` as JSON, and where it comes from.
fn effective(tree: &SettingsTree, scope: Scope, key: SettingKey) -> (Value, Source) {
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
fn input_id(key: SettingKey) -> String {
    format!("in-{}", key.name())
}

/// The input for one setting.
fn input(key: SettingKey, scope: Scope, value: &Value, locale: Locale, disabled: bool) -> AnyView {
    let id = input_id(key);
    let s = shown(value);
    match key.meta().kind {
        FieldKind::Bool => {
            let on = value.as_bool().unwrap_or(false);
            view! {
                <select id=id name="value" disabled=disabled>
                    <option value="on" selected=on>{text(locale, "ui-on", &[])}</option>
                    <option value="off" selected=!on>{text(locale, "ui-off", &[])}</option>
                </select>
            }
            .into_any()
        }
        FieldKind::Probability => view! {
            <input id=id name="value" type="number" min="0" max="1" step="0.01" value=s disabled=disabled/>
        }
        .into_any(),
        FieldKind::Count => view! {
            <input id=id name="value" type="number" min="1" step="1" value=s disabled=disabled/>
        }
        .into_any(),
        FieldKind::Rate => view! {
            <input id=id name="value" type="number" min="0.1" step="0.05" value=s disabled=disabled/>
        }
        .into_any(),
        FieldKind::Number { unit } => view! {
            <input id=id name="value" type="number" step="any" value=s disabled=disabled/>
            <span class="unit">{unit}</span>
        }
        .into_any(),
        FieldKind::Duration { unlimited, .. } => {
            let hint = if unlimited {
                text(locale, "ui-duration-or-unlimited", &[])
            } else {
                text(locale, "ui-duration", &[])
            };
            view! { <input id=id name="value" type="text" value=s placeholder=hint.clone() title=hint disabled=disabled/> }
                .into_any()
        }
        FieldKind::Choice { choices } => {
            view! {
                <select id=id name="value" disabled=disabled>
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
                <select id=id name="value" disabled=disabled>
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
        FieldKind::TimeOfDay => view! { <input id=id name="value" type="time" value=s disabled=disabled/> }.into_any(),
        FieldKind::Origin => view! {
            <input id=id name="value" type="url" value=s placeholder="https://bot.example.org" disabled=disabled/>
        }
        .into_any(),
        FieldKind::Lang => view! {
            <select id=id name="value" disabled=disabled><LangOptions selected=s locale/></select>
        }
        .into_any(),
        FieldKind::VoiceLang => view! {
            <select id=id name="value" disabled=disabled><LangOptions selected=s auto=true locale/></select>
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
                                    <select name=format!("voice.{}", k.as_str()) disabled=disabled>
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
                <input id=id name="value" type="text" value=s list="time-zones" autocomplete="off" disabled=disabled/>
                <datalist id="time-zones">
                    {zones.into_iter().map(|z| view! { <option value=z></option> }).collect_view()}
                </datalist>
            }
            .into_any()
        }
        FieldKind::Ids { of } => id_picker(of, scope, value, locale, disabled),
        FieldKind::Hosts | FieldKind::Langs | FieldKind::Prefix => view! {
            <input id=id name="value" type="text" value=s disabled=disabled/>
        }
        .into_any(),
    }
}

/// The ids in a list setting's value.
fn ids_of(value: &Value) -> Vec<u64> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_u64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
                .collect()
        })
        .unwrap_or_default()
}

/// What can be picked for a list of communities, roles or people, by name: (id, name, where it is from).
fn id_choices(of: &str, scope: Scope, chosen: &[u64]) -> Vec<(u64, String, Option<String>)> {
    let gs = app().engine.guilds();
    let mut out: Vec<(u64, String, Option<String>)> = match of {
        "community" => {
            let mut ids: std::collections::BTreeSet<u64> = gs.guilds.keys().map(|g| g.0).collect();
            ids.extend(gs.known_communities.keys().map(|g| g.0));
            ids.into_iter()
                .map(|g| (g, gs.guild_name(pb_domain::GuildId(g)), None))
                .collect()
        }
        // The roles of the scope's community, or of every community (each named with its community).
        "role" => gs
            .guilds
            .iter()
            .filter(|(g, _)| scope.guild().is_none_or(|s| s == **g))
            .flat_map(|(g, info)| {
                let mut roles: Vec<_> = info.roles.values().filter(|r| r.id.0 != g.0).collect();
                roles.sort_by_key(|r| std::cmp::Reverse(r.position));
                let community = scope.guild().is_none().then(|| info.name.clone());
                roles
                    .into_iter()
                    .map(move |r| (r.id.0, r.name.clone(), community.clone()))
                    .collect::<Vec<_>>()
            })
            .collect(),
        // People are many: the chosen ones as boxes (named from any community the bot saw them in); others are
        // added from the list of everyone the bot knows (see `people_to_add`).
        _ => chosen
            .iter()
            .map(|u| {
                let user = pb_domain::UserId(*u);
                let name = gs
                    .guilds
                    .iter()
                    .find_map(|(_, i)| i.people.get(&user).map(pb_engine::Person::shown))
                    .or_else(|| gs.known.iter().find(|((_, x), _)| *x == user).map(|(_, p)| p.shown()))
                    .unwrap_or_else(|| u.to_string());
                (*u, name, None)
            })
            .collect(),
    };
    // Chosen ids the bot does not know by name stay choosable.
    for id in chosen {
        if !out.iter().any(|(x, _, _)| x == id) {
            out.push((*id, id.to_string(), None));
        }
    }
    out
}

/// Everyone the bot knows by name who is not chosen yet (people, not bots), by name.
fn people_to_add(chosen: &[u64]) -> Vec<(u64, String)> {
    let gs = app().engine.guilds();
    let mut seen = std::collections::BTreeMap::new();
    for info in gs.guilds.values() {
        for p in info.people.values().filter(|p| !p.bot) {
            seen.entry(p.user.0).or_insert_with(|| p.shown());
        }
    }
    for ((_, u), p) in &gs.known {
        if !p.bot {
            seen.entry(u.0).or_insert_with(|| p.shown());
        }
    }
    let mut out: Vec<(u64, String)> = seen.into_iter().filter(|(u, _)| !chosen.contains(u)).collect();
    out.sort_by_key(|a| a.1.to_lowercase());
    out
}

/// A list of communities, roles or people, picked by name (a box each), plus a field for ids typed or pasted.
fn id_picker(of: &str, scope: Scope, value: &Value, locale: Locale, disabled: bool) -> AnyView {
    let chosen = ids_of(value);
    let choices = id_choices(of, scope, &chosen);
    let add = (of == "user").then(|| people_to_add(&chosen));
    let placeholder = text(
        locale,
        match of {
            "community" => "ui-ids-more-communities",
            "role" => "ui-ids-more-roles",
            _ => "ui-ids-more-people",
        },
        &[],
    );
    view! {
        <div class="id-picker">
            {(choices.is_empty() && of != "user").then(|| view! { <p class="muted small">{text(locale, "ui-ids-none-known", &[])}</p> })}
            {choices.into_iter().map(|(id, name, from)| {
                let on = chosen.contains(&id);
                view! {
                    <label class="inline">
                        <input type="checkbox" name="value" value=id.to_string() checked=on disabled=disabled/>
                        {name}
                        {from.map(|f| view! { <span class="muted small">{f}</span> })}
                    </label>
                }
            }).collect_view()}
            <div class="row">
                {add.filter(|a| !a.is_empty()).map(|people| view! {
                    <select name="value" disabled=disabled aria-label=text(locale, "ui-ids-add-person", &[])>
                        <option value="">{text(locale, "ui-ids-add-person", &[])}</option>
                        {people.into_iter().map(|(u, name)| view! { <option value=u.to_string()>{name}</option> }).collect_view()}
                    </select>
                })}
                <input name="value" type="text" placeholder=placeholder.clone() title=placeholder disabled=disabled/>
            </div>
        </div>
    }
    .into_any()
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
fn advanced(key: SettingKey) -> bool {
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
            | K::SpeechRate
            | K::NoSpeakPolicy
            | K::CommandPrefix
            | K::Instance
            | K::AllowedHosts
            | K::CpuThreads
            | K::TtsThreads
    )
}

/// Settings that only work together with another one: (this one, the other one). Where the other one cannot be set
/// at the scope shown, the row says where it lives, with its value there and a link to it.
const WORKS_WITH: &[(SettingKey, SettingKey)] = &[
    // Audio in the mod log needs the mod-log channel, which is chosen per community.
    (SettingKey::ModlogAudio, SettingKey::ModlogChannel),
];

/// A value as a short text for people (a channel by its name, nothing as "not set").
fn value_label(key: SettingKey, guild: Option<pb_domain::GuildId>, value: &Value, loc: Locale) -> String {
    match (key.meta().kind, value) {
        (_, Value::Null) => text(loc, "ui-not-set", &[]),
        (_, Value::String(s)) if s.is_empty() => text(loc, "ui-not-set", &[]),
        (_, Value::Bool(on)) => text(loc, if *on { "ui-on" } else { "ui-off" }, &[]),
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
                let (value, _) = effective(&tree, Scope::Server { guild: g }, other);
                (
                    format!("/c/{g}/settings#{anchor}"),
                    gs.guild_name(g),
                    value_label(other, Some(g), &value, loc),
                )
            })
            .collect();
        ("ui-where-per-community", links)
    } else if kinds.contains(&ScopeKind::Global) {
        let (value, _) = effective(&tree, Scope::Global, other);
        let links = viewer
            .owner
            .then(|| {
                (
                    format!("/system#{anchor}"),
                    text(loc, "ui-nav-system", &[]),
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
/// when it is set here, and where a setting it works together with lives when that is set elsewhere.
#[component]
fn SettingRow(key: SettingKey, scope: Scope, viewer: Viewer, back: String) -> impl IntoView {
    let loc = viewer.locale;
    let tree = app().engine.settings().current();
    let meta = key.meta();
    let here = tree.overrides(scope).get_json(key);
    let (value, source) = effective(&tree, scope, key);
    let disabled = meta.who == Who::Owner && !viewer.owner;
    let is_here = here.is_some();
    let badge = if is_here {
        text(loc, "ui-set-here", &[])
    } else {
        text(
            loc,
            "ui-inherited",
            &[("from", text(loc, source_id(source), &[]).into())],
        )
    };
    let partner = WORKS_WITH
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, other)| *other)
        .filter(|other| !other.scopes().contains(&scope.kind()))
        .map(|other| works_with(other, scope, &viewer));
    let anchor = format!("set-{}", key.name());
    // Saving comes back to this setting (with "Advanced" open when it is in there).
    let back = if advanced(key) {
        format!("{back}?advanced=1#{anchor}")
    } else {
        format!("{back}#{anchor}")
    };
    view! {
        <form method="post" action="/settings" class="setting" id=anchor.clone() class:here=is_here>
            <input type="hidden" name="csrf" value=viewer.csrf.clone()/>
            <input type="hidden" name="scope" value=scope_param(scope)/>
            <input type="hidden" name="key" value=key.name()/>
            <input type="hidden" name="back" value=back/>
            <div class="setting-head">
                <label for=input_id(key)>{setting_name(loc, key)}</label>
                <details class="help">
                    <summary title=text(loc, "ui-help", &[])>"?"</summary>
                    <p>{setting_help(loc, key)}</p>
                </details>
                <span class="badge" class:here=is_here>{badge}</span>
                {(meta.who == Who::Owner).then(|| view! { <span class="badge owner">{text(loc, "ui-owner-only", &[])}</span> })}
                {super::commands::setting_commands(key, scope, loc)}
            </div>
            <div class="row">
                {input(key, scope, &value, loc, disabled)}
                {(!disabled).then(|| view! {
                    <button class="button" name="action" value="set">{text(loc, "ui-save", &[])}</button>
                })}
                {(is_here && !disabled).then(|| view! {
                    <button class="link" name="action" value="clear">{text(loc, "ui-use-inherited", &[])}</button>
                })}
            </div>
            {partner}
        </form>
    }
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

/// `keys` grouped by section (sections without any left out).
fn by_section(keys: &[SettingKey]) -> Vec<(Section, Vec<SettingKey>)> {
    Section::ALL
        .into_iter()
        .filter_map(|section| {
            let mine: Vec<SettingKey> = keys.iter().copied().filter(|k| k.meta().section == section).collect();
            (!mine.is_empty()).then(|| (section, ordered(mine)))
        })
        .collect()
}

/// Every setting that can be set at `scope`: the everyday ones by section, then "Advanced" (folded), then resetting
/// everything set here.
#[component]
pub fn SettingsForm(scope: Scope, back: String) -> impl IntoView {
    let Some(v) = viewer() else {
        return ().into_any();
    };
    let loc = v.locale;
    let kind = scope.kind();
    let keys: Vec<SettingKey> = SettingKey::all()
        .into_iter()
        .filter(|k| k.scopes().contains(&kind))
        .collect();
    let (more, basic): (Vec<SettingKey>, Vec<SettingKey>) = keys.into_iter().partition(|k| advanced(*k));
    let sections = by_section(&basic);
    let more = by_section(&more);
    let open = use_query_map().with_untracked(|q| q.get("advanced").is_some());
    let row = |key: SettingKey| view! { <SettingRow key scope viewer=v.clone() back=back.clone()/> };
    // A menu of the sections first: the page is long.
    let menu = sections
        .iter()
        .map(|(section, _)| {
            view! { <a href=format!("#section-{}", section.key())>{section_name(loc, *section)}</a> }
        })
        .collect_view();
    let cards = sections
        .into_iter()
        .map(|(section, keys)| {
            // The chat commands' reference beside their settings (on the System page).
            let commands = (section == Section::Commands && scope == Scope::Global)
                .then(|| view! { <super::commands::ChatCommands guild=None locale=loc/> });
            view! {
                <section class="card settings-section" id=format!("section-{}", section.key())>
                    <h2>{section_name(loc, section)}</h2>
                    {keys.into_iter().map(row).collect_view()}
                    {commands}
                </section>
            }
        })
        .collect_view();
    let advanced_card = (!more.is_empty()).then(|| {
        view! {
            <details class="card settings-section advanced" id="advanced" open=open>
                <summary><h2>{text(loc, "ui-advanced", &[])}</h2></summary>
                <p class="muted small">{text(loc, "ui-advanced-help", &[])}</p>
                {more.into_iter().map(|(section, keys)| view! {
                    <h3>{section_name(loc, section)}</h3>
                    {keys.into_iter().map(row).collect_view()}
                }).collect_view()}
            </details>
        }
    });
    let resettable = resettable(scope, v.owner).len();
    let reset = (resettable > 0).then(|| {
        view! {
            <section class="card settings-reset">
                <form method="post" action="/settings/reset" class="row">
                    <input type="hidden" name="csrf" value=v.csrf.clone()/>
                    <input type="hidden" name="scope" value=scope_param(scope)/>
                    <input type="hidden" name="back" value=back.clone()/>
                    <span class="grow muted">{text(loc, "ui-reset-settings-help", &[("count", resettable.into())])}</span>
                    <button class="button danger">{text(loc, "ui-reset-settings", &[])}</button>
                </form>
            </section>
        }
    });
    view! {
        <nav class="settings-menu">
            {menu}
            {advanced_card.is_some().then(|| view! { <a href="#advanced">{text(loc, "ui-advanced", &[])}</a> })}
        </nav>
        {cards}
        {advanced_card}
        {reset}
    }
    .into_any()
}
