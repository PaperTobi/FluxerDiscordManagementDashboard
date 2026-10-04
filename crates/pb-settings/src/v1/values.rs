//! Setting value types. Each validates when it is parsed, so a value that exists is valid.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use pb_domain::Lang;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! via_str {
    ($t:ty) => {
        impl Serialize for $t {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $t {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

/// Why a value was refused. Shown in the reader's language in the web UI and chat (`pb_i18n::value_error`); the
/// English text here is for settings files and logs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueError {
    #[error("{0} is not between 0 and 1")]
    NotProbability(String),
    #[error("must be at least 1")]
    BelowOne,
    #[error("{0} is not a whole number")]
    NotWhole(String),
    #[error("{0} is too large")]
    TooLarge(String),
    #[error("{0:?} is not a length of time")]
    NotDuration(String),
    #[error("{0:?} is negative")]
    Negative(String),
    #[error("must be longer than zero")]
    NotPositive,
    #[error("must be at least 32 ms (one audio frame)")]
    BelowFrame,
    #[error("must be a finite number")]
    NotFinite,
    #[error("must be above zero")]
    NotAboveZero,
    #[error("{0:?} must look like 09:00")]
    NotTimeOfDay(String),
    #[error("unknown time zone {0:?} (use a name like Europe/Berlin)")]
    UnknownTz(String),
    #[error("{0:?} is not an http(s) address")]
    NotOrigin(String),
    #[error("{0:?} must be just the address, like http://192.168.1.50:8790 (no path)")]
    OriginWithPath(String),
    #[error("{0:?} is not a host name")]
    NotHost(String),
    #[error("the prefix may not contain spaces")]
    PrefixSpaces,
    #[error("{0:?} is not a language tag (like de or en-US)")]
    NotLang(String),
    #[error("{0:?} is not an ID")]
    NotId(String),
    #[error("{value:?} must be one of: {choices}")]
    NotChoice { value: String, choices: String },
    #[error("escalation needs at least one step")]
    NoSteps,
    #[error("step {0}: steps must start at increasing violation counts (1, 2, 3 …)")]
    StepOrder(usize),
    #[error("step {0}: Fluxer allows time-outs of at most 365.25 days")]
    TimeoutTooLong(usize),
    #[error("step {step}: {error}")]
    Step { step: usize, error: Box<ValueError> },
    #[error("{0:?} is not a number")]
    NotNumber(String),
    #[error("must be on or off")]
    NotSwitch,
    #[error("must be text")]
    NotText,
    #[error("must be a list")]
    NotList,
    #[error("unknown field {0:?}")]
    UnknownField(String),
}

/// Reading a setting's value from JSON (what the web UI and chat commands send) with a [`ValueError`] that says why
/// a value was refused. Settings files go through serde with the same checks.
pub trait FromJson: Sized {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError>;
}

fn number(v: &serde_json::Value) -> Result<f64, ValueError> {
    match v {
        serde_json::Value::Number(n) => n.as_f64().ok_or_else(|| ValueError::NotNumber(n.to_string())),
        serde_json::Value::String(s) => s
            .trim()
            .replace(',', ".")
            .parse()
            .map_err(|_| ValueError::NotNumber(s.clone())),
        other => Err(ValueError::NotNumber(other.to_string())),
    }
}

fn text_of(v: serde_json::Value) -> Result<String, ValueError> {
    match v {
        serde_json::Value::String(s) => Ok(s),
        serde_json::Value::Number(n) => Ok(n.to_string()),
        _ => Err(ValueError::NotText),
    }
}

/// A switch typed as a word, in English or German (`on`/`an`, `off`/`aus`, `yes`/`ja`, `no`/`nein`, `true`, `false`,
/// `1`, `0`).
pub fn switch_word(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" | "an" | "ja" => Some(true),
        "off" | "false" | "no" | "0" | "aus" | "nein" => Some(false),
        _ => None,
    }
}

impl FromJson for bool {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        match v {
            serde_json::Value::Bool(b) => Ok(b),
            serde_json::Value::String(s) => switch_word(&s).ok_or(ValueError::NotSwitch),
            _ => Err(ValueError::NotSwitch),
        }
    }
}

impl<T: FromJson> FromJson for Option<T> {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        match v {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(ref s) if s.trim().is_empty() => Ok(None),
            other => T::from_json(other).map(Some),
        }
    }
}

impl<T: FromJson> FromJson for Vec<T> {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        match v {
            serde_json::Value::Array(a) => a.into_iter().map(T::from_json).collect(),
            serde_json::Value::Null => Ok(Vec::new()),
            _ => Err(ValueError::NotList),
        }
    }
}

impl FromJson for Lang {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        let s = text_of(v)?;
        s.parse().map_err(|_| ValueError::NotLang(s))
    }
}

macro_rules! id_from_json {
    ($($t:ty),*) => {$(
        impl FromJson for $t {
            fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
                let s = text_of(v)?;
                s.trim().parse::<$t>().map_err(|_| ValueError::NotId(s))
            }
        }
    )*};
}
id_from_json!(
    pb_domain::GuildId,
    pb_domain::UserId,
    pb_domain::RoleId,
    pb_domain::ChannelId
);

impl FromJson for std::collections::BTreeMap<Lang, String> {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        match v {
            serde_json::Value::Object(o) => o
                .into_iter()
                .map(|(k, v)| {
                    let l: Lang = k.parse().map_err(|_| ValueError::NotLang(k))?;
                    Ok((l, text_of(v)?))
                })
                .collect(),
            serde_json::Value::Null => Ok(Self::new()),
            _ => Err(ValueError::NotList),
        }
    }
}

/// Text values that parse with `FromStr` (and their `ValueError`).
macro_rules! text_from_json {
    ($($t:ty),*) => {$(
        impl FromJson for $t {
            fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
                text_of(v)?.parse()
            }
        }
    )*};
}

/// A probability strictly between 0 and 1.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Probability(f64);

impl Probability {
    pub fn new(v: f64) -> Result<Self, ValueError> {
        if v.is_finite() && v > 0.0 && v < 1.0 {
            Ok(Probability(v))
        } else {
            Err(ValueError::NotProbability(v.to_string()))
        }
    }
    pub fn get(self) -> f64 {
        self.0
    }
}

impl FromJson for Probability {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        Probability::new(number(&v)?)
    }
}

impl<'de> Deserialize<'de> for Probability {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Probability::new(f64::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// A count of at least 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Count(u32);

impl Count {
    pub fn new(v: u32) -> Result<Self, ValueError> {
        if v >= 1 {
            Ok(Count(v))
        } else {
            Err(ValueError::BelowOne)
        }
    }
    pub fn get(self) -> u32 {
        self.0
    }
}

impl FromJson for Count {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        let n = number(&v)?;
        if n.fract() != 0.0 {
            return Err(ValueError::NotWhole(n.to_string()));
        }
        if n < 0.0 {
            return Err(ValueError::Negative(n.to_string()));
        }
        if n > f64::from(u32::MAX) {
            return Err(ValueError::TooLarge(n.to_string()));
        }
        Count::new(n as u32)
    }
}

impl<'de> Deserialize<'de> for Count {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Count::new(u32::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// A length of time. Written as text (`"20s"`, `"1.5s"`, `"600ms"`, `"1h 30m"`) or a number of seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Dur(Duration);

impl Dur {
    pub fn from_secs(s: f64) -> Result<Self, ValueError> {
        if s < 0.0 {
            return Err(ValueError::Negative(s.to_string()));
        }
        Duration::try_from_secs_f64(s)
            .map(Dur)
            .map_err(|_| ValueError::NotDuration(s.to_string()))
    }
    pub const fn from_millis(ms: u64) -> Self {
        Dur(Duration::from_millis(ms))
    }
    pub fn get(self) -> Duration {
        self.0
    }
    pub fn secs(self) -> f64 {
        self.0.as_secs_f64()
    }
    pub fn millis(self) -> u64 {
        self.0.as_millis() as u64
    }
}

impl FromStr for Dur {
    type Err = ValueError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if let Ok(secs) = t.parse::<f64>() {
            return Dur::from_secs(secs);
        }
        // "20s", "1h 30m", and also days and weeks ("2d", "1w"; a day counts as 24 hours here).
        let d: jiff::SignedDuration = match t.parse() {
            Ok(d) => d,
            Err(_) => t
                .parse::<jiff::Span>()
                .and_then(|span| span.to_duration(jiff::SpanRelativeTo::days_are_24_hours()))
                .map_err(|_| ValueError::NotDuration(t.to_owned()))?,
        };
        Duration::try_from(d)
            .map(Dur)
            .map_err(|_| ValueError::Negative(t.to_owned()))
    }
}

impl FromJson for Dur {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        match v {
            serde_json::Value::Number(n) => Dur::from_secs(n.as_f64().unwrap_or(f64::NAN)),
            other => text_of(other)?.parse(),
        }
    }
}

impl fmt::Display for Dur {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let d = jiff::SignedDuration::try_from(self.0).map_err(|_| fmt::Error)?;
        if d.is_zero() {
            f.write_str("0s")
        } else {
            write!(f, "{d:#}")
        }
    }
}

impl Serialize for Dur {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Dur {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(f64),
            Text(String),
        }
        match Raw::deserialize(d)? {
            Raw::Num(n) => Dur::from_secs(n),
            Raw::Text(t) => t.parse(),
        }
        .map_err(serde::de::Error::custom)
    }
}

/// A length of time above zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct PosDur(Dur);

impl PosDur {
    pub fn new(d: Dur) -> Result<Self, ValueError> {
        if d.get().is_zero() {
            Err(ValueError::NotPositive)
        } else {
            Ok(PosDur(d))
        }
    }
    pub const fn from_millis(ms: u64) -> Self {
        PosDur(Dur::from_millis(ms))
    }
    pub fn get(self) -> Dur {
        self.0
    }
}

impl FromJson for PosDur {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        PosDur::new(Dur::from_json(v)?)
    }
}

impl<'de> Deserialize<'de> for PosDur {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        PosDur::new(Dur::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// At least one 32 ms audio frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct FrameDur(Dur);

impl FrameDur {
    pub fn new(d: Dur) -> Result<Self, ValueError> {
        if d.millis() < 32 {
            Err(ValueError::BelowFrame)
        } else {
            Ok(FrameDur(d))
        }
    }
    pub const fn from_millis(ms: u64) -> Self {
        FrameDur(Dur::from_millis(ms))
    }
    pub fn get(self) -> Dur {
        self.0
    }
}

impl FromJson for FrameDur {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        FrameDur::new(Dur::from_json(v)?)
    }
}

impl<'de> Deserialize<'de> for FrameDur {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        FrameDur::new(Dur::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// A value or "unlimited".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Limit<T> {
    Unlimited,
    Value(T),
}

impl<T> Limit<T> {
    pub fn value(self) -> Option<T> {
        match self {
            Limit::Unlimited => None,
            Limit::Value(v) => Some(v),
        }
    }
}

impl<T: FromJson> FromJson for Limit<T> {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        if let Some(s) = v.as_str()
            && matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "unlimited" | "inf" | "∞" | "none"
            )
        {
            return Ok(Limit::Unlimited);
        }
        T::from_json(v).map(Limit::Value)
    }
}

impl<T: Serialize> Serialize for Limit<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Limit::Unlimited => s.serialize_str("unlimited"),
            Limit::Value(v) => v.serialize(s),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Limit<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        if let Some(s) = v.as_str()
            && matches!(
                s.trim().to_ascii_lowercase().as_str(),
                "unlimited" | "inf" | "∞" | "none"
            )
        {
            return Ok(Limit::Unlimited);
        }
        T::deserialize(v).map(Limit::Value).map_err(serde::de::Error::custom)
    }
}

/// A finite number (decibels, speeds).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Finite(f64);

impl Finite {
    pub fn new(v: f64) -> Result<Self, ValueError> {
        if v.is_finite() {
            Ok(Finite(v))
        } else {
            Err(ValueError::NotFinite)
        }
    }
    pub fn get(self) -> f64 {
        self.0
    }
}

impl FromJson for Finite {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        Finite::new(number(&v)?)
    }
}

impl<'de> Deserialize<'de> for Finite {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Finite::new(f64::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// A speaking speed above zero (1.0 = the voice's own pace).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Rate(f64);

impl Rate {
    pub fn new(v: f64) -> Result<Self, ValueError> {
        if v.is_finite() && v > 0.0 {
            Ok(Rate(v))
        } else {
            Err(ValueError::NotAboveZero)
        }
    }
    pub fn get(self) -> f64 {
        self.0
    }
}

impl FromJson for Rate {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        Rate::new(number(&v)?)
    }
}

impl<'de> Deserialize<'de> for Rate {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Rate::new(f64::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

/// Time of day `HH:MM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TimeOfDay {
    pub hour: u8,
    pub minute: u8,
}

impl FromStr for TimeOfDay {
    type Err = ValueError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ValueError::NotTimeOfDay(s.to_owned());
        let (h, m) = s.trim().split_once(':').ok_or_else(err)?;
        let digits = |t: &str| t.len() == 2 && t.bytes().all(|b| b.is_ascii_digit());
        if !digits(h) || !digits(m) {
            return Err(err());
        }
        let hour: u8 = h.parse().map_err(|_| err())?;
        let minute: u8 = m.parse().map_err(|_| err())?;
        if hour > 23 || minute > 59 {
            return Err(err());
        }
        Ok(TimeOfDay { hour, minute })
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)
    }
}
via_str!(TimeOfDay);
text_from_json!(TimeOfDay);

/// An IANA time zone name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tz(String);

impl Tz {
    pub fn zone(&self) -> jiff::tz::TimeZone {
        jiff::tz::TimeZone::get(&self.0).unwrap_or(jiff::tz::TimeZone::UTC)
    }
}

impl FromStr for Tz {
    type Err = ValueError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let t = if t.is_empty() { "UTC" } else { t };
        jiff::tz::TimeZone::get(t)
            .map(|_| Tz(t.to_owned()))
            .map_err(|_| ValueError::UnknownTz(t.to_owned()))
    }
}

impl fmt::Display for Tz {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
via_str!(Tz);
text_from_json!(Tz);

/// A Fluxer instance's address: its API address or its web address, with or without a path
/// (`https://api.fluxer.app`, `https://example.com/api`, `example.com`; https is assumed when no scheme is given).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstanceUrl(url::Url);

impl InstanceUrl {
    pub fn url(&self) -> &url::Url {
        &self.0
    }
}

impl FromStr for InstanceUrl {
    type Err = ValueError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let with_scheme = if t.contains("://") {
            t.to_owned()
        } else {
            format!("https://{t}")
        };
        let u = url::Url::parse(&with_scheme).map_err(|_| ValueError::NotOrigin(t.to_owned()))?;
        if !matches!(u.scheme(), "http" | "https")
            || u.host_str().is_none_or(str::is_empty)
            || u.query().is_some()
            || u.fragment().is_some()
        {
            return Err(ValueError::NotOrigin(t.to_owned()));
        }
        Ok(InstanceUrl(u))
    }
}

impl fmt::Display for InstanceUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str().trim_end_matches('/'))
    }
}
via_str!(InstanceUrl);
text_from_json!(InstanceUrl);

/// An http(s) address with nothing after the host (origin), e.g. the web UI address.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Origin(url::Url);

impl Origin {
    pub fn url(&self) -> &url::Url {
        &self.0
    }
    pub fn host(&self) -> Option<&str> {
        self.0.host_str()
    }
}

impl FromStr for Origin {
    type Err = ValueError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim().trim_end_matches('/');
        let u = url::Url::parse(t).map_err(|_| ValueError::NotOrigin(t.to_owned()))?;
        if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
            return Err(ValueError::NotOrigin(t.to_owned()));
        }
        if u.path() != "/" || u.query().is_some() || u.fragment().is_some() {
            return Err(ValueError::OriginWithPath(t.to_owned()));
        }
        Ok(Origin(u))
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str().trim_end_matches('/'))
    }
}
via_str!(Origin);
text_from_json!(Origin);

/// A host name (or IP address) people may open the web UI by.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostName(String);

impl HostName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for HostName {
    type Err = ValueError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim().to_ascii_lowercase();
        let ok = !t.is_empty()
            && t.split('.')
                .all(|label| !label.is_empty() && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
            || t.parse::<std::net::IpAddr>().is_ok();
        if ok {
            Ok(HostName(t))
        } else {
            Err(ValueError::NotHost(s.to_owned()))
        }
    }
}

impl fmt::Display for HostName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
via_str!(HostName);
text_from_json!(HostName);

/// The chat command prefix: any text without spaces (empty = only mentions of the bot work).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Prefix(String);

impl Prefix {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Prefix {
    type Err = ValueError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        if t.chars().any(char::is_whitespace) {
            Err(ValueError::PrefixSpaces)
        } else {
            Ok(Prefix(t.to_owned()))
        }
    }
}

impl fmt::Display for Prefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
via_str!(Prefix);
text_from_json!(Prefix);

/// The language the bot speaks to a person: a fixed one, or the one the classifier heard them speak.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VoiceLang {
    Fixed(Lang),
    Auto,
}

impl FromStr for VoiceLang {
    type Err = ValueError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.trim().eq_ignore_ascii_case("auto") {
            Ok(VoiceLang::Auto)
        } else {
            s.parse()
                .map(VoiceLang::Fixed)
                .map_err(|_| ValueError::NotLang(s.to_owned()))
        }
    }
}

impl fmt::Display for VoiceLang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoiceLang::Fixed(l) => fmt::Display::fmt(l, f),
            VoiceLang::Auto => f.write_str("auto"),
        }
    }
}
via_str!(VoiceLang);
text_from_json!(VoiceLang);

macro_rules! choice {
    ($(#[$doc:meta])* $name:ident { $($(#[$vdoc:meta])* $variant:ident = $text:literal),+ $(,)? }) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name { $($(#[$vdoc])* $variant),+ }
        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$variant),+];
            pub fn as_str(self) -> &'static str { match self { $($name::$variant => $text),+ } }
        }
        impl FromStr for $name {
            type Err = ValueError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let t = s.trim().to_ascii_lowercase();
                $name::ALL.iter().copied().find(|v| v.as_str() == t).ok_or_else(|| ValueError::NotChoice {
                    value: s.to_owned(),
                    choices: $name::ALL.iter().map(|v| v.as_str()).collect::<Vec<_>>().join(", "),
                })
            }
        }
        text_from_json!($name);
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.as_str()) }
        }
        via_str!($name);
    };
}

choice!(
    /// What happens when the bot may not speak in a channel.
    NoSpeakPolicy { Text = "text", Log = "log" }
);
choice!(
    /// The owner's summary report.
    Digest { Off = "off", Daily = "daily", Weekly = "weekly" }
);
choice!(
    /// Day of the week.
    Weekday { Monday = "monday", Tuesday = "tuesday", Wednesday = "wednesday", Thursday = "thursday", Friday = "friday", Saturday = "saturday", Sunday = "sunday" }
);
choice!(
    /// Which sentences of tracked people are kept as recordings.
    Recordings { Off = "off", Flagged = "flagged", All = "all" }
);
choice!(
    /// Who hears the bot.
    AudienceChoice { Offender = "offender", Tracked = "tracked", Channel = "channel" }
);
choice!(
    /// The action of an escalation step.
    StepAction { None = "none", Mute = "mute", Disconnect = "disconnect", Timeout = "timeout" }
);

impl StepAction {
    /// What the bot does to the member (`None`: nothing).
    pub fn kind(self) -> Option<pb_domain::ActionKind> {
        match self {
            StepAction::None => None,
            StepAction::Mute => Some(pb_domain::ActionKind::Mute),
            StepAction::Disconnect => Some(pb_domain::ActionKind::Disconnect),
            StepAction::Timeout => Some(pb_domain::ActionKind::Timeout),
        }
    }
}

impl From<AudienceChoice> for pb_domain::Audience {
    fn from(a: AudienceChoice) -> Self {
        match a {
            AudienceChoice::Offender => pb_domain::Audience::Offender,
            AudienceChoice::Tracked => pb_domain::Audience::Tracked,
            AudienceChoice::Channel => pb_domain::Audience::Channel,
        }
    }
}

/// One escalation step: from the `from`-th violation in the counting window on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscalationStep {
    pub from: Count,
    /// A direct message to the bot owner (the mod log, when set, gets every flagged sentence anyway).
    #[serde(default)]
    pub notify_owner: bool,
    #[serde(default = "no_action")]
    pub action: StepAction,
    /// How long a mute or timeout lasts.
    #[serde(default = "five_minutes")]
    pub duration: PosDur,
}

fn no_action() -> StepAction {
    StepAction::None
}

fn five_minutes() -> PosDur {
    PosDur::from_millis(5 * 60 * 1000)
}

/// The longest time-out Fluxer allows (365.25 days; `TIMEOUT_CANNOT_EXCEED_365_DAYS`).
pub const MAX_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(365 * 86_400 + 6 * 3600);

/// The escalation steps, starting at increasing violation counts (at least one step).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Escalation(Vec<EscalationStep>);

impl Escalation {
    pub fn new(steps: Vec<EscalationStep>) -> Result<Self, ValueError> {
        if steps.is_empty() {
            return Err(ValueError::NoSteps);
        }
        let mut last = 0;
        for (i, s) in steps.iter().enumerate() {
            if s.from.get() <= last {
                return Err(ValueError::StepOrder(i + 1));
            }
            last = s.from.get();
            if s.action == StepAction::Timeout && s.duration.get().get() > MAX_TIMEOUT {
                return Err(ValueError::TimeoutTooLong(i + 1));
            }
        }
        Ok(Escalation(steps))
    }

    pub fn steps(&self) -> &[EscalationStep] {
        &self.0
    }

    /// The step for the `count`-th violation (1-based index of the step, and the step).
    pub fn step_for(&self, count: u32) -> Option<(u32, &EscalationStep)> {
        self.0
            .iter()
            .enumerate()
            .rev()
            .find(|(_, s)| count >= s.from.get())
            .map(|(i, s)| (i as u32 + 1, s))
    }

    /// The old bot's three steps (warnings only), each telling the owner (as its default did for every violation).
    pub fn default_steps() -> Self {
        let step = |from| EscalationStep {
            from: Count(from),
            notify_owner: true,
            action: StepAction::None,
            duration: five_minutes(),
        };
        Escalation(vec![step(1), step(2), step(3)])
    }
}

impl FromJson for EscalationStep {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        let serde_json::Value::Object(mut o) = v else {
            return Err(ValueError::NotList);
        };
        if let Some(k) = o
            .keys()
            .find(|k| !matches!(k.as_str(), "from" | "notify_owner" | "action" | "duration"))
        {
            return Err(ValueError::UnknownField(k.clone()));
        }
        let mut take = |k: &str| o.remove(k);
        Ok(EscalationStep {
            from: Count::from_json(take("from").unwrap_or(serde_json::Value::Null))?,
            notify_owner: take("notify_owner").map_or(Ok(false), bool::from_json)?,
            action: take("action").map_or(Ok(StepAction::None), StepAction::from_json)?,
            duration: take("duration").map_or(Ok(five_minutes()), PosDur::from_json)?,
        })
    }
}

impl FromJson for Escalation {
    fn from_json(v: serde_json::Value) -> Result<Self, ValueError> {
        let serde_json::Value::Array(a) = v else {
            return Err(ValueError::NotList);
        };
        let steps = a
            .into_iter()
            .enumerate()
            .map(|(i, s)| {
                EscalationStep::from_json(s).map_err(|e| ValueError::Step {
                    step: i + 1,
                    error: Box::new(e),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Escalation::new(steps)
    }
}

impl<'de> Deserialize<'de> for Escalation {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Escalation::new(Vec::deserialize(d)?).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_of_day_are_two_digits_each() {
        assert_eq!("09:05".parse::<TimeOfDay>().map(|t| (t.hour, t.minute)), Ok((9, 5)));
        for bad in ["+9:00", "9:00", "09:+5", "24:00", "12:60"] {
            assert!(bad.parse::<TimeOfDay>().is_err(), "{bad}");
        }
    }

    #[test]
    fn json_values_say_why_they_are_refused() {
        use serde_json::json;
        assert_eq!(
            Probability::from_json(json!(2)),
            Err(ValueError::NotProbability("2".into()))
        );
        assert_eq!(Probability::from_json(json!("0,4")).map(Probability::get), Ok(0.4));
        assert_eq!(Count::from_json(json!(0)), Err(ValueError::BelowOne));
        assert_eq!(Count::from_json(json!(1.5)), Err(ValueError::NotWhole("1.5".into())));
        assert_eq!(Count::from_json(json!(-2)), Err(ValueError::Negative("-2".into())));
        assert_eq!(
            Dur::from_json(json!("soon")),
            Err(ValueError::NotDuration("soon".into()))
        );
        assert_eq!(PosDur::from_json(json!("0s")), Err(ValueError::NotPositive));
        assert_eq!(FrameDur::from_json(json!("10ms")), Err(ValueError::BelowFrame));
        assert_eq!(Limit::<PosDur>::from_json(json!("unlimited")), Ok(Limit::Unlimited));
        assert_eq!(
            TimeOfDay::from_json(json!("9am")),
            Err(ValueError::NotTimeOfDay("9am".into()))
        );
        assert_eq!(
            Tz::from_json(json!("Mars/Base")),
            Err(ValueError::UnknownTz("Mars/Base".into()))
        );
        assert_eq!(
            Origin::from_json(json!("http://x/y")),
            Err(ValueError::OriginWithPath("http://x/y".into()))
        );
        assert!(matches!(
            AudienceChoice::from_json(json!("loud")),
            Err(ValueError::NotChoice { .. })
        ));
        assert_eq!(bool::from_json(json!("maybe")), Err(ValueError::NotSwitch));
        assert_eq!(
            Vec::<pb_domain::UserId>::from_json(json!(["x"])),
            Err(ValueError::NotId("x".into()))
        );
        let steps = json!([{"from": 2}, {"from": 1}]);
        assert_eq!(Escalation::from_json(steps), Err(ValueError::StepOrder(2)));
        let steps = json!([{"from": 1, "action": "timeout", "duration": "400d"}]);
        assert_eq!(Escalation::from_json(steps), Err(ValueError::TimeoutTooLong(1)));
        let steps = json!([{"from": 1, "colour": "red"}]);
        assert_eq!(
            Escalation::from_json(steps),
            Err(ValueError::Step {
                step: 1,
                error: Box::new(ValueError::UnknownField("colour".into()))
            })
        );
    }

    #[test]
    fn durations_read_and_write_as_text() {
        assert_eq!("20s".parse::<Dur>().expect("20s").secs(), 20.0);
        assert_eq!("1.5s".parse::<Dur>().expect("1.5s").millis(), 1500);
        assert_eq!("600ms".parse::<Dur>().expect("600ms").millis(), 600);
        assert_eq!("1h 30m".parse::<Dur>().expect("1h30").secs(), 5400.0);
        assert_eq!("2.5".parse::<Dur>().expect("number").millis(), 2500);
        assert_eq!("2d".parse::<Dur>().expect("days").secs(), 2.0 * 86_400.0);
        assert_eq!("1w 1d".parse::<Dur>().expect("weeks").secs(), 8.0 * 86_400.0);
        assert_eq!(Dur::from_millis(90_000).to_string(), "1m 30s");
        assert!("-5s".parse::<Dur>().is_err());
        let l: Limit<PosDur> = serde_json::from_value(serde_json::json!("unlimited")).expect("unlimited");
        assert_eq!(l, Limit::Unlimited);
        let l: Limit<PosDur> = serde_json::from_value(serde_json::json!("20s")).expect("20s");
        assert_eq!(l.value().map(|d| d.get().secs()), Some(20.0));
        assert!(serde_json::from_value::<PosDur>(serde_json::json!(0)).is_err());
    }

    #[test]
    fn rejects_meaningless_values_only() {
        assert!(
            Probability::new(0.0).is_err() && Probability::new(1.0).is_err() && Probability::new(0.999_999).is_ok()
        );
        assert!(Count::new(0).is_err() && Count::new(1_000_000).is_ok());
        assert!("09:00".parse::<TimeOfDay>().is_ok() && "24:00".parse::<TimeOfDay>().is_err());
        assert!("Europe/Berlin".parse::<Tz>().is_ok() && "Mars/Base".parse::<Tz>().is_err());
        assert!("https://fivius.com/".parse::<Origin>().is_ok() && "https://fivius.com/x".parse::<Origin>().is_err());
        assert!("!pb".parse::<Prefix>().is_ok() && "a b".parse::<Prefix>().is_err());
        assert!("botbox.lan".parse::<HostName>().is_ok() && "a b".parse::<HostName>().is_err());
    }

    #[test]
    fn escalation_steps_increase() {
        let e = Escalation::default_steps();
        assert_eq!(e.step_for(1).map(|s| s.0), Some(1));
        assert_eq!(e.step_for(7).map(|s| s.0), Some(3));
        assert_eq!(e.step_for(0), None);
        let bad = serde_json::json!([{"from": 2}, {"from": 2}]);
        assert!(serde_json::from_value::<Escalation>(bad).is_err());
    }

    #[test]
    fn instance_addresses_may_have_a_path_and_leave_out_https() {
        for (given, want) in [
            ("https://api.fluxer.app", "https://api.fluxer.app"),
            ("https://fivius.com/api/", "https://fivius.com/api"),
            ("fivius.com/api", "https://fivius.com/api"),
            ("http://192.168.1.50:8080", "http://192.168.1.50:8080"),
        ] {
            assert_eq!(
                given.parse::<InstanceUrl>().map(|u| u.to_string()),
                Ok(want.to_owned()),
                "{given}"
            );
        }
        for bad in ["ftp://x.example", "https://x.example/?a=1", "https://", ""] {
            assert!(bad.parse::<InstanceUrl>().is_err(), "{bad}");
        }
    }
}
