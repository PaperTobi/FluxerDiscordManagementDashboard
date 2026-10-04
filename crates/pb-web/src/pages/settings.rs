//! The settings of one scope (global, a community, a person), grouped by section. Each setting is its own small form:
//! the value, where the value in effect comes from, Save, and "Use inherited" when it is set here. Owner-only
//! settings are shown to admins but not editable.

use leptos::prelude::*;
use pb_domain::{Label, Scope};
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

/// The input for one setting.
fn input(key: SettingKey, scope: Scope, value: &Value, locale: Locale, disabled: bool) -> AnyView {
    let id = format!("set-{}", key.name());
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
        FieldKind::Ids { .. } | FieldKind::Hosts | FieldKind::Langs | FieldKind::Prefix => view! {
            <input id=id name="value" type="text" value=s disabled=disabled/>
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

/// One setting.
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
    let anchor = format!("set-{}", key.name());
    view! {
        <form method="post" action="/settings" class="setting" id=anchor.clone() class:here=is_here>
            <input type="hidden" name="csrf" value=viewer.csrf.clone()/>
            <input type="hidden" name="scope" value=scope_param(scope)/>
            <input type="hidden" name="key" value=key.name()/>
            <input type="hidden" name="back" value=format!("{back}#{anchor}")/>
            <div class="setting-head">
                <label for=anchor.clone()>{setting_name(loc, key)}</label>
                <span class="badge" class:here=is_here>{badge}</span>
                {(meta.who == Who::Owner).then(|| view! { <span class="badge owner">{text(loc, "ui-owner-only", &[])}</span> })}
            </div>
            <p class="help">{setting_help(loc, key)}</p>
            <div class="row">
                {input(key, scope, &value, loc, disabled)}
                {(!disabled).then(|| view! {
                    <button class="button" name="action" value="set">{text(loc, "ui-save", &[])}</button>
                })}
                {(is_here && !disabled).then(|| view! {
                    <button class="link" name="action" value="clear">{text(loc, "ui-use-inherited", &[])}</button>
                })}
            </div>
        </form>
    }
}

/// Every setting that can be set at `scope`, by section.
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
    let sections: Vec<(Section, Vec<SettingKey>)> = Section::ALL
        .into_iter()
        .filter_map(|section| {
            let mine: Vec<SettingKey> = keys.iter().copied().filter(|k| k.meta().section == section).collect();
            if mine.is_empty() {
                return None;
            }
            // Per-type switches and bars after the general ones, in the order of the detection types.
            let (general, mut per_label): (Vec<_>, Vec<_>) = mine
                .into_iter()
                .partition(|k| !matches!(k, SettingKey::LabelEnabled(_) | SettingKey::LabelThreshold(_)));
            per_label.sort_by_key(|k| match k {
                SettingKey::LabelEnabled(l) => (Label::ALL.iter().position(|x| x == l), 0),
                SettingKey::LabelThreshold(l) => (Label::ALL.iter().position(|x| x == l), 1),
                _ => (None, 2),
            });
            Some((section, general.into_iter().chain(per_label).collect()))
        })
        .collect();
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
            let rows = keys
                .into_iter()
                .map(|key| view! { <SettingRow key scope viewer=v.clone() back=back.clone()/> })
                .collect_view();
            view! {
                <section class="card settings-section" id=format!("section-{}", section.key())>
                    <h2>{section_name(loc, section)}</h2>
                    {rows}
                </section>
            }
        })
        .collect_view();
    view! {
        <nav class="settings-menu">{menu}</nav>
        {cards}
    }
    .into_any()
}
