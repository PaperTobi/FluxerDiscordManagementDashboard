//! Version 1.

use std::sync::LazyLock;
use std::time::Duration;

use fluent_bundle::concurrent::FluentBundle;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue};
use pb_domain::{Label, Lang};
use pb_settings::{Section, SettingError, SettingKey};
use serde::{Deserialize, Serialize};

/// A language the bot's own text is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    En,
    De,
}

impl Locale {
    pub const ALL: [Locale; 2] = [Locale::En, Locale::De];

    pub const fn tag(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::De => "de",
        }
    }

    /// The locale for a language setting: its language if the bot writes it, otherwise English.
    pub fn for_lang(lang: &Lang) -> Locale {
        Locale::ALL
            .into_iter()
            .find(|l| l.tag() == lang.language())
            .unwrap_or(Locale::En)
    }

    /// The locale for an HTTP `Accept-Language` header: the supported entry with the highest weight, else English.
    pub fn negotiate(accept_language: &str) -> Locale {
        let mut best: Option<(f32, Locale)> = None;
        for part in accept_language.split(',') {
            let mut it = part.split(';');
            let tag = it.next().unwrap_or_default().trim();
            let q = it
                .find_map(|p| p.trim().strip_prefix("q="))
                .and_then(|q| q.parse::<f32>().ok())
                .unwrap_or(1.0);
            let Ok(lang) = tag.parse::<Lang>() else { continue };
            let Some(loc) = Locale::ALL.into_iter().find(|l| l.tag() == lang.language()) else {
                continue;
            };
            if q > 0.0 && best.is_none_or(|(bq, _)| q > bq) {
                best = Some((q, loc));
            }
        }
        best.map_or(Locale::En, |(_, l)| l)
    }

    /// A number with at most `digits` decimals (trailing zeros dropped) and the locale's decimal mark.
    pub fn decimal(self, v: f64, digits: usize) -> String {
        let mut s = format!("{v:.digits$}");
        if s.contains('.') {
            s = s.trim_end_matches('0').trim_end_matches('.').to_owned();
        }
        if s == "-0" {
            s = "0".into();
        }
        self.mark(s)
    }

    /// A number with exactly `digits` decimals (scores read better aligned).
    pub fn fixed(self, v: f64, digits: usize) -> String {
        self.mark(format!("{v:.digits$}"))
    }

    fn mark(self, s: String) -> String {
        match self {
            Locale::En => s,
            Locale::De => s.replace('.', ","),
        }
    }
}

/// A message argument.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Text(String),
    Number(f64),
}

impl From<&str> for Arg {
    fn from(s: &str) -> Self {
        Arg::Text(s.to_owned())
    }
}

impl From<String> for Arg {
    fn from(s: String) -> Self {
        Arg::Text(s)
    }
}

impl From<&String> for Arg {
    fn from(s: &String) -> Self {
        Arg::Text(s.clone())
    }
}

impl From<bool> for Arg {
    /// Booleans select `[yes]` / `[no]` variants.
    fn from(b: bool) -> Self {
        Arg::Text(if b { "yes" } else { "no" }.to_owned())
    }
}

macro_rules! number_arg {
    ($($t:ty),*) => {$(
        impl From<$t> for Arg {
            fn from(n: $t) -> Self { Arg::Number(n as f64) }
        }
    )*};
}
number_arg!(u8, u16, u32, u64, usize, i32, i64, f32, f64);

macro_rules! sources {
    ($($file:literal),* $(,)?) => {
        &[
            $((Locale::En, $file, include_str!(concat!("../../locales/en/", $file))),)*
            $((Locale::De, $file, include_str!(concat!("../../locales/de/", $file))),)*
        ]
    };
}

/// The compiled-in sources: (locale, file name, text).
pub const SOURCES: &[(Locale, &str, &str)] = sources!("bot.ftl", "settings.ftl", "ui.ftl", "setup.ftl");

/// A problem found while loading or formatting.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Problem(pub String);

/// A message id with the names of its attributes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageId {
    pub id: String,
    pub attributes: Vec<String>,
}

struct Loaded {
    locale: Locale,
    bundle: FluentBundle<FluentResource>,
    ids: Vec<MessageId>,
}

/// Every message in every locale.
pub struct Catalog {
    loaded: Vec<Loaded>,
    problems: Vec<Problem>,
}

impl std::fmt::Debug for Catalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Catalog")
            .field("locales", &self.loaded.iter().map(|l| l.locale).collect::<Vec<_>>())
            .field("problems", &self.problems)
            .finish()
    }
}

static CATALOG: LazyLock<Catalog> = LazyLock::new(|| Catalog::load(SOURCES));

impl Catalog {
    /// The built-in catalog.
    pub fn get() -> &'static Catalog {
        &CATALOG
    }

    /// Loads sources. Syntax errors and duplicate ids do not stop loading (the rest still works); they are kept in
    /// [`Catalog::problems`], which the tests require to be empty for the built-in files.
    pub fn load(sources: &[(Locale, &str, &str)]) -> Catalog {
        let mut problems = Vec::new();
        let mut loaded: Vec<Loaded> = Vec::new();
        for locale in Locale::ALL {
            let id: unic_langid::LanguageIdentifier = locale.tag().parse().unwrap_or_default();
            let mut bundle = FluentBundle::new_concurrent(vec![id]);
            // Mentions like <@123> must reach Fluxer untouched, so no bidi isolation marks around arguments.
            bundle.set_use_isolating(false);
            let mut ids = Vec::new();
            for (_, file, text) in sources.iter().filter(|s| s.0 == locale) {
                let res = match FluentResource::try_new((*text).to_owned()) {
                    Ok(r) => r,
                    Err((r, errs)) => {
                        problems.extend(errs.iter().map(|e| Problem(format!("{}/{file}: {e:?}", locale.tag()))));
                        r
                    }
                };
                for entry in res.entries() {
                    if let fluent_syntax::ast::Entry::Message(m) = entry {
                        ids.push(MessageId {
                            id: m.id.name.to_owned(),
                            attributes: m.attributes.iter().map(|a| a.id.name.to_owned()).collect(),
                        });
                    }
                }
                if let Err(errs) = bundle.add_resource(res) {
                    problems.extend(errs.iter().map(|e| Problem(format!("{}/{file}: {e}", locale.tag()))));
                }
            }
            loaded.push(Loaded { locale, bundle, ids });
        }
        Catalog { loaded, problems }
    }

    pub fn problems(&self) -> &[Problem] {
        &self.problems
    }

    fn loaded(&self, locale: Locale) -> Option<&Loaded> {
        self.loaded.iter().find(|l| l.locale == locale)
    }

    /// Message ids with their attributes, in file order.
    pub fn ids(&self, locale: Locale) -> &[MessageId] {
        self.loaded(locale).map_or(&[], |l| l.ids.as_slice())
    }

    pub fn has(&self, locale: Locale, id: &str) -> bool {
        self.loaded(locale).is_some_and(|l| l.bundle.has_message(id))
    }

    /// Formats a message or one of its attributes, reporting every problem (an unknown id, a missing argument).
    pub fn try_format(
        &self,
        locale: Locale,
        id: &str,
        attr: Option<&str>,
        args: &[(&str, Arg)],
    ) -> Result<String, Vec<Problem>> {
        let Some((out, errors)) = self.format(locale, id, attr, args) else {
            return Err(vec![Problem(format!(
                "{}: no {id}{}",
                locale.tag(),
                attr.map(|a| format!(".{a}")).unwrap_or_default()
            ))]);
        };
        if errors.is_empty() {
            Ok(out)
        } else {
            Err(errors
                .into_iter()
                .map(|e| Problem(format!("{}: {id}: {e}", locale.tag())))
                .collect())
        }
    }

    /// A message or attribute formatted, with the problems met on the way; `None` when there is no such text.
    fn format(
        &self,
        locale: Locale,
        id: &str,
        attr: Option<&str>,
        args: &[(&str, Arg)],
    ) -> Option<(String, Vec<fluent_bundle::FluentError>)> {
        let bundle = &self.loaded(locale)?.bundle;
        let msg = bundle.get_message(id)?;
        let pattern = match attr {
            None => msg.value(),
            Some(a) => msg.get_attribute(a).map(|a| a.value()),
        }?;
        let fargs = fluent_args(args);
        let mut errors = Vec::new();
        let out = bundle.format_pattern(pattern, Some(&fargs), &mut errors).into_owned();
        Some((out, errors))
    }

    /// Formats a message. One missing in `locale` falls back to English; one missing everywhere shows its id. (The
    /// tests make both impossible for built-in ids.)
    pub fn text(&self, locale: Locale, id: &str, args: &[(&str, Arg)]) -> String {
        self.lenient(locale, id, None, args)
    }

    /// Formats an attribute of a message (like `.help`), with the fallbacks of [`Catalog::text`].
    pub fn attr(&self, locale: Locale, id: &str, attr: &str, args: &[(&str, Arg)]) -> String {
        self.lenient(locale, id, Some(attr), args)
    }

    fn lenient(&self, locale: Locale, id: &str, attr: Option<&str>, args: &[(&str, Arg)]) -> String {
        let locale = if self.has(locale, id) { locale } else { Locale::En };
        self.format(locale, id, attr, args)
            .map_or_else(|| id.to_owned(), |(out, _)| out)
    }
}

fn fluent_args<'a>(args: &'a [(&'a str, Arg)]) -> FluentArgs<'a> {
    let mut out = FluentArgs::with_capacity(args.len());
    for (k, v) in args {
        match v {
            Arg::Text(s) => out.set(*k, FluentValue::from(s.as_str())),
            Arg::Number(n) => out.set(*k, FluentValue::from(*n)),
        }
    }
    out
}

/// Formats a built-in message.
pub fn text(locale: Locale, id: &str, args: &[(&str, Arg)]) -> String {
    Catalog::get().text(locale, id, args)
}

/// A setting's value as people read it (`null` is "unlimited": the only setting values that can be empty are limits).
pub fn value_text(locale: Locale, v: &serde_json::Value) -> String {
    use serde_json::Value;
    match v {
        Value::Null => text(locale, "dur-unlimited", &[]),
        Value::Bool(b) => text(locale, if *b { "value-on" } else { "value-off" }, &[]),
        Value::Number(n) => n.as_f64().map_or_else(|| n.to_string(), |f| locale.decimal(f, 3)),
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .map(|x| value_text(locale, x))
            .collect::<Vec<_>>()
            .join(", "),
        Value::Object(fields) => fields
            .iter()
            .map(|(k, x)| format!("{k}: {}", value_text(locale, x)))
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// The message id of a setting (`strike_window` → `setting-strike-window`); per-label keys share one id each.
pub fn setting_id(key: SettingKey) -> String {
    match key {
        SettingKey::LabelEnabled(_) => "setting-label-enabled".into(),
        SettingKey::LabelThreshold(_) => "setting-label-threshold".into(),
        k => format!("setting-{}", k.name().replace('_', "-")),
    }
}

fn setting_args(locale: Locale, key: SettingKey) -> Vec<(&'static str, Arg)> {
    match key {
        SettingKey::LabelEnabled(l) | SettingKey::LabelThreshold(l) => {
            vec![("label", Arg::Text(label_name(locale, l)))]
        }
        _ => Vec::new(),
    }
}

/// A setting's name.
pub fn setting_name(locale: Locale, key: SettingKey) -> String {
    Catalog::get().text(locale, &setting_id(key), &setting_args(locale, key))
}

/// A setting's help text.
pub fn setting_help(locale: Locale, key: SettingKey) -> String {
    Catalog::get().attr(locale, &setting_id(key), "help", &setting_args(locale, key))
}

pub fn section_id(section: Section) -> String {
    format!("section-{}", section.key())
}

pub fn section_name(locale: Locale, section: Section) -> String {
    text(locale, &section_id(section), &[])
}

pub fn label_id(label: Label) -> String {
    format!("label-{}", label.key())
}

/// A detection type's name.
pub fn label_name(locale: Locale, label: Label) -> String {
    text(locale, &label_id(label), &[])
}

/// A Fluxer permission's name as Fluxer shows it, from its stable name (`pb_fluxer_api::perms::NAMES`).
pub fn permission_name(locale: Locale, name: &str) -> String {
    text(locale, &format!("perm-{name}"), &[])
}

/// A moderation action, with how long it lasts when it does ("mute for 5 minutes").
pub fn action_text(locale: Locale, kind: pb_domain::ActionKind, secs: Option<u64>) -> String {
    let id = match kind {
        pb_domain::ActionKind::Mute => "action-mute",
        pb_domain::ActionKind::Unmute => "action-unmute",
        pb_domain::ActionKind::Disconnect => "action-disconnect",
        pb_domain::ActionKind::Timeout => "action-timeout",
    };
    let action = text(locale, id, &[]);
    match secs {
        Some(s) => text(
            locale,
            "action-for",
            &[
                ("action", action.into()),
                ("duration", duration(locale, Some(Duration::from_secs(s))).into()),
            ],
        ),
        None => action,
    }
}

/// How a moderation action went.
pub fn action_outcome(locale: Locale, outcome: &pb_domain::ActionOutcome) -> String {
    use pb_domain::ActionOutcome as O;
    match outcome {
        O::Done => text(locale, "action-done", &[]),
        O::SkippedOff => text(locale, "action-skipped-off", &[]),
        O::SkippedObserve => text(locale, "action-skipped-observe", &[]),
        O::AlreadyMuted => text(locale, "action-already-muted", &[]),
        O::NotConnected => text(locale, "action-not-connected", &[]),
        O::NotAllowed { permission } => text(
            locale,
            "action-not-allowed",
            &[("permission", permission_name(locale, permission).into())],
        ),
        O::Failed { error } => text(locale, "action-failed", &[("error", error.clone().into())]),
    }
}

/// The message id of a setting's choice (`digest_weekday` shares the weekday names).
pub fn setting_choice_id(key: SettingKey, value: &str) -> String {
    let kind = match key {
        SettingKey::DigestWeekday => "weekday".to_owned(),
        k => k.name(),
    };
    choice_id(&kind, value)
}

/// A setting's value as people read it, in `locale`: switches as on/off, choices by name, limits as "unlimited".
pub fn setting_value(locale: Locale, key: SettingKey, v: &serde_json::Value) -> String {
    use pb_settings::FieldKind;
    match (key.meta().kind, v) {
        (FieldKind::Choice { .. }, serde_json::Value::String(c)) => text(locale, &setting_choice_id(key, c), &[]),
        (FieldKind::Duration { unlimited: true, .. }, serde_json::Value::String(d)) if d == "unlimited" => {
            text(locale, "dur-unlimited", &[])
        }
        _ => value_text(locale, v),
    }
}

/// The message id of a choice: `choice-<kind>-<value>` with `_` written as `-`, e.g. `choice-no-speak-policy-text`.
pub fn choice_id(kind: &str, value: &str) -> String {
    format!("choice-{}-{}", kind.replace('_', "-"), value.replace('_', "-"))
}

/// A length of time in the largest unit that keeps it readable (`1.5 s`, `20 min`, `3 days`); `None` is unlimited.
pub fn duration(locale: Locale, d: Option<Duration>) -> String {
    let Some(d) = d else {
        return text(locale, "dur-unlimited", &[]);
    };
    let (unit, n) = in_units(d);
    length(locale, &format!("dur-{unit}"), n)
}

/// A length of time as words to be spoken ("5 minutes", "1,5 Stunden").
pub fn spoken_duration(locale: Locale, d: Duration) -> String {
    let (unit, n) = in_units(d);
    length(locale, &format!("spoken-{unit}"), n)
}

/// The unit a length of time reads best in, and the number of them rounded to one decimal (59.96 s is 1 min, not
/// 60 s).
fn in_units(d: Duration) -> (&'static str, f64) {
    const UNITS: [(&str, f64, f64); 5] = [
        ("ms", 1.0, 1000.0),
        ("s", 1000.0, 60.0),
        ("min", 60_000.0, 60.0),
        ("h", 3_600_000.0, 24.0),
        ("d", 86_400_000.0, f64::INFINITY),
    ];
    let ms = d.as_secs_f64() * 1000.0;
    for (unit, size, next) in UNITS {
        let n = (ms / size * 10.0).round() / 10.0;
        if n < next {
            return (unit, n);
        }
    }
    ("d", (ms / 86_400_000.0 * 10.0).round() / 10.0)
}

fn length(locale: Locale, id: &str, n: f64) -> String {
    text(
        locale,
        id,
        &[("n", Arg::Number(n)), ("shown", Arg::Text(locale.decimal(n, 1)))],
    )
}

/// Why a value was refused, in `locale`.
pub fn value_error(locale: Locale, e: &pb_settings::ValueError) -> String {
    use pb_settings::ValueError as E;
    let v = |id: &str, value: &str| text(locale, id, &[("value", value.to_owned().into())]);
    match e {
        E::NotProbability(x) => v("err-not-probability", x),
        E::BelowOne => text(locale, "err-below-one", &[]),
        E::NotWhole(x) => v("err-not-whole", x),
        E::TooLarge(x) => v("err-too-large", x),
        E::NotDuration(x) => v("err-not-duration", x),
        E::Negative(x) => v("err-negative", x),
        E::NotPositive => text(locale, "err-not-positive", &[]),
        E::BelowFrame => text(locale, "err-below-frame", &[]),
        E::NotFinite => text(locale, "err-not-finite", &[]),
        E::NotAboveZero => text(locale, "err-not-above-zero", &[]),
        E::NotTimeOfDay(x) => v("err-not-time-of-day", x),
        E::UnknownTz(x) => v("err-unknown-tz", x),
        E::NotOrigin(x) => v("err-not-origin", x),
        E::OriginWithPath(x) => v("err-origin-with-path", x),
        E::NotHost(x) => v("err-not-host", x),
        E::PrefixSpaces => text(locale, "err-prefix-spaces", &[]),
        E::NotLang(x) => v("err-not-lang", x),
        E::NotLineKind(x) => v("err-not-line-kind", x),
        E::NotId(x) => v("err-not-id", x),
        E::NotChoice { value, choices } => text(
            locale,
            "err-not-choice",
            &[("value", value.clone().into()), ("choices", choices.clone().into())],
        ),
        E::NoSteps => text(locale, "err-no-steps", &[]),
        E::StepOrder(n) => text(locale, "err-step-order", &[("step", (*n).into())]),
        E::TimeoutTooLong(n) => text(locale, "err-timeout-too-long", &[("step", (*n).into())]),
        E::Step { step, error } => text(
            locale,
            "err-step",
            &[("step", (*step).into()), ("problem", value_error(locale, error).into())],
        ),
        E::NotNumber(x) => v("err-not-number", x),
        E::NotSwitch => text(locale, "err-not-switch", &[]),
        E::NotText => text(locale, "err-not-text", &[]),
        E::NotList => text(locale, "err-not-list", &[]),
        E::UnknownField(x) => v("err-unknown-field", x),
    }
}

/// Why a setting change was refused, in `locale`.
pub fn setting_error(locale: Locale, e: &SettingError) -> String {
    let name = |k: &str| {
        k.parse::<SettingKey>()
            .map_or_else(|_| k.to_owned(), |k| setting_name(locale, k))
    };
    match e {
        SettingError::Unknown(k) => text(locale, "err-unknown-setting", &[("name", k.clone().into())]),
        SettingError::Scope { key, scope } => {
            let scope = match scope {
                pb_domain::ScopeKind::Global => "global",
                pb_domain::ScopeKind::Server => "server",
                pb_domain::ScopeKind::Person => "person",
            };
            text(
                locale,
                "err-scope",
                &[("setting", name(key).into()), ("scope", scope.into())],
            )
        }
        SettingError::Invalid { key, error } => text(
            locale,
            "err-setting",
            &[
                ("setting", name(key).into()),
                ("problem", value_error(locale, error).into()),
            ],
        ),
        SettingError::OwnerOnly { key } => text(locale, "err-owner-only", &[("setting", name(key).into())]),
    }
}
