//! The declaration of every setting and what is generated from it.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use pb_domain::{ChannelId, GuildId, Label, Lang, RoleId, ScopeKind, UserId};
use pb_voicelines::LineKind;
use serde::{Deserialize, Serialize};

use super::values::*;

/// Where a resolved value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Person,
    Server,
    Global,
    /// `config.toml` (`[defaults]`) or an environment variable.
    File,
    Default,
}

/// A resolved value and where it came from.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Resolved<T> {
    pub value: T,
    pub source: Source,
}

/// When a change takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Apply {
    /// At once.
    Live,
    /// After the bot reconnects to Fluxer (the web UI does that by itself).
    Reconnect,
}

/// Who may change a setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Who {
    /// The bot owner and community admins (in their community).
    Admins,
    /// Only the bot owner.
    Owner,
}

/// Sections of the settings page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Tracking,
    Detection,
    Warning,
    Escalation,
    Reporting,
    Recording,
    Commands,
    System,
}

impl Section {
    pub const ALL: [Section; 8] = [
        Section::Tracking,
        Section::Detection,
        Section::Warning,
        Section::Escalation,
        Section::Reporting,
        Section::Recording,
        Section::Commands,
        Section::System,
    ];

    /// The name in files and URLs.
    pub const fn key(self) -> &'static str {
        match self {
            Section::Tracking => "tracking",
            Section::Detection => "detection",
            Section::Warning => "warning",
            Section::Escalation => "escalation",
            Section::Reporting => "reporting",
            Section::Recording => "recording",
            Section::Commands => "commands",
            Section::System => "system",
        }
    }
}

/// The input a value needs in a form.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FieldKind {
    Bool,
    Probability,
    Count,
    Duration {
        min_ms: u64,
        unlimited: bool,
    },
    Number {
        unit: &'static str,
    },
    Rate,
    Choice {
        choices: Vec<&'static str>,
    },
    Ids {
        of: &'static str,
    },
    Channel,
    TimeOfDay,
    Tz,
    Origin,
    Hosts,
    Prefix,
    Lang,
    Langs,
    VoiceLang,
    Voices,
    /// A voice per kind of line.
    LineVoices,
    Escalation,
}

/// What the web UI and docs know about a setting.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FieldMeta {
    pub key: String,
    pub section: Section,
    pub scopes: Vec<ScopeKind>,
    pub who: Who,
    pub apply: Apply,
    #[serde(flatten)]
    pub kind: FieldKind,
    /// The built-in default, as JSON.
    pub default: serde_json::Value,
}

/// A setting value type: how it is edited.
pub trait SettingType: Clone + PartialEq + fmt::Debug + Serialize + for<'de> Deserialize<'de> + FromJson {
    fn kind() -> FieldKind;
    /// For optional settings: "empty", which clears the setting at that scope instead of storing a value.
    fn is_unset(&self) -> bool {
        false
    }
}

impl<T: SettingType> SettingType for Option<T> {
    fn kind() -> FieldKind {
        T::kind()
    }
    fn is_unset(&self) -> bool {
        self.is_none()
    }
}

macro_rules! kind {
    ($t:ty => $k:expr) => {
        impl SettingType for $t {
            fn kind() -> FieldKind {
                $k
            }
        }
    };
}

kind!(bool => FieldKind::Bool);
kind!(Probability => FieldKind::Probability);
kind!(Count => FieldKind::Count);
kind!(Dur => FieldKind::Duration { min_ms: 0, unlimited: false });
kind!(PosDur => FieldKind::Duration { min_ms: 1, unlimited: false });
kind!(FrameDur => FieldKind::Duration { min_ms: 32, unlimited: false });
kind!(Limit<PosDur> => FieldKind::Duration { min_ms: 1, unlimited: true });
kind!(Finite => FieldKind::Number { unit: "dB" });
kind!(Rate => FieldKind::Rate);
kind!(AudienceChoice => FieldKind::Choice { choices: AudienceChoice::ALL.iter().map(|c| c.as_str()).collect() });
kind!(NoSpeakPolicy => FieldKind::Choice { choices: NoSpeakPolicy::ALL.iter().map(|c| c.as_str()).collect() });
kind!(Digest => FieldKind::Choice { choices: Digest::ALL.iter().map(|c| c.as_str()).collect() });
kind!(Weekday => FieldKind::Choice { choices: Weekday::ALL.iter().map(|c| c.as_str()).collect() });
kind!(Recordings => FieldKind::Choice { choices: Recordings::ALL.iter().map(|c| c.as_str()).collect() });
kind!(Vec<GuildId> => FieldKind::Ids { of: "community" });
kind!(Vec<UserId> => FieldKind::Ids { of: "user" });
kind!(Vec<RoleId> => FieldKind::Ids { of: "role" });
kind!(ChannelId => FieldKind::Channel);
kind!(TimeOfDay => FieldKind::TimeOfDay);
kind!(Tz => FieldKind::Tz);
kind!(Origin => FieldKind::Origin);
kind!(InstanceUrl => FieldKind::Origin);
kind!(Vec<HostName> => FieldKind::Hosts);
kind!(Prefix => FieldKind::Prefix);
kind!(Lang => FieldKind::Lang);
kind!(Vec<Lang> => FieldKind::Langs);
kind!(VoiceLang => FieldKind::VoiceLang);
kind!(BTreeMap<Lang, String> => FieldKind::Voices);
kind!(BTreeMap<LineKind, String> => FieldKind::LineVoices);
kind!(Escalation => FieldKind::Escalation);

/// A detection type's own settings at one scope.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LabelLayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Empty = the general threshold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<Probability>,
}

const ALL_SCOPES: &[ScopeKind] = &[ScopeKind::Global, ScopeKind::Server, ScopeKind::Person];
const GS: &[ScopeKind] = &[ScopeKind::Global, ScopeKind::Server];
const G: &[ScopeKind] = &[ScopeKind::Global];
const S: &[ScopeKind] = &[ScopeKind::Server];

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum SettingError {
    #[error("unknown setting {0:?}")]
    Unknown(String),
    #[error("{key} cannot be set {}", where_(*scope))]
    Scope { key: String, scope: ScopeKind },
    #[error("{key}: {error}")]
    Invalid { key: String, error: ValueError },
    #[error("{key} can only be changed by the bot owner")]
    OwnerOnly { key: String },
}

fn where_(scope: ScopeKind) -> &'static str {
    match scope {
        ScopeKind::Global => "globally",
        ScopeKind::Server => "per community",
        ScopeKind::Person => "for a person",
    }
}

macro_rules! settings {
    ($(
        $section:ident {
            $( $field:ident ( $variant:ident ) : $ty:ty = $default:expr ; $scopes:ident, $who:ident, $apply:ident ; )*
        }
    )*) => {
        /// Every setting (plus the per-detection-type ones).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum SettingKey {
            $($($variant,)*)*
            LabelEnabled(Label),
            LabelThreshold(Label),
        }

        impl SettingKey {
            /// Every key, in display order.
            pub fn all() -> Vec<SettingKey> {
                let mut out = vec![$($(SettingKey::$variant,)*)*];
                for l in Label::ALL {
                    out.push(SettingKey::LabelEnabled(l));
                    out.push(SettingKey::LabelThreshold(l));
                }
                out
            }

            pub fn name(self) -> String {
                match self {
                    $($(SettingKey::$variant => stringify!($field).to_owned(),)*)*
                    SettingKey::LabelEnabled(l) => format!("label.{}.enabled", l.key()),
                    SettingKey::LabelThreshold(l) => format!("label.{}.threshold", l.key()),
                }
            }

            pub fn meta(self) -> FieldMeta {
                let label_meta = |default: serde_json::Value, kind| FieldMeta {
                    key: self.name(), section: Section::Detection, scopes: ALL_SCOPES.to_vec(), who: Who::Admins,
                    apply: Apply::Live, kind, default,
                };
                match self {
                    $($(SettingKey::$variant => FieldMeta {
                        key: stringify!($field).to_owned(),
                        section: Section::$section,
                        scopes: $scopes.to_vec(),
                        who: Who::$who,
                        apply: Apply::$apply,
                        kind: <$ty as SettingType>::kind(),
                        default: serde_json::to_value(&Defaults::builtin().$field).unwrap_or(serde_json::Value::Null),
                    },)*)*
                    SettingKey::LabelEnabled(l) => label_meta(serde_json::Value::Bool(enabled_by_default(l)), FieldKind::Bool),
                    SettingKey::LabelThreshold(_) => label_meta(serde_json::Value::Null, FieldKind::Probability),
                }
            }

            pub fn scopes(self) -> &'static [ScopeKind] {
                match self {
                    $($(SettingKey::$variant => $scopes,)*)*
                    SettingKey::LabelEnabled(_) | SettingKey::LabelThreshold(_) => ALL_SCOPES,
                }
            }

            pub fn who(self) -> Who {
                match self {
                    $($(SettingKey::$variant => Who::$who,)*)*
                    SettingKey::LabelEnabled(_) | SettingKey::LabelThreshold(_) => Who::Admins,
                }
            }

            pub fn apply(self) -> Apply {
                match self {
                    $($(SettingKey::$variant => Apply::$apply,)*)*
                    SettingKey::LabelEnabled(_) | SettingKey::LabelThreshold(_) => Apply::Live,
                }
            }
        }

        impl FromStr for SettingKey {
            type Err = SettingError;
            fn from_str(s: &str) -> Result<Self, SettingError> {
                match s {
                    $($(stringify!($field) => return Ok(SettingKey::$variant),)*)*
                    _ => {}
                }
                let parts: Vec<&str> = s.split('.').collect();
                if let ["label", l, what] = parts.as_slice() {
                    if let Ok(label) = l.parse::<Label>() {
                        match *what {
                            "enabled" => return Ok(SettingKey::LabelEnabled(label)),
                            "threshold" => return Ok(SettingKey::LabelThreshold(label)),
                            _ => {}
                        }
                    }
                }
                Err(SettingError::Unknown(s.to_owned()))
            }
        }

        impl fmt::Display for SettingKey {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.name())
            }
        }

        impl Serialize for SettingKey {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.name())
            }
        }

        impl<'de> Deserialize<'de> for SettingKey {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }

        /// The settings set at one scope (or in `config.toml`). Missing = not set here.
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct Layer {
            $($(
                #[serde(skip_serializing_if = "Option::is_none")]
                pub $field: Option<$ty>,
            )*)*
            /// Per detection type.
            #[serde(skip_serializing_if = "BTreeMap::is_empty")]
            pub labels: BTreeMap<Label, LabelLayer>,
        }

        impl Layer {
            /// The value set here for `key`, as JSON (`None` = not set here).
            pub fn get_json(&self, key: SettingKey) -> Option<serde_json::Value> {
                match key {
                    $($(SettingKey::$variant => self.$field.as_ref().and_then(|v| serde_json::to_value(v).ok()),)*)*
                    SettingKey::LabelEnabled(l) => self.labels.get(&l).and_then(|x| x.enabled).map(serde_json::Value::Bool),
                    SettingKey::LabelThreshold(l) => self.labels.get(&l).and_then(|x| x.threshold).and_then(|v| serde_json::to_value(v).ok()),
                }
            }

            /// Sets `key` from JSON (validating it). Returns the stored value as JSON.
            pub(crate) fn set_json(&mut self, key: SettingKey, value: serde_json::Value) -> Result<serde_json::Value, SettingError> {
                let invalid = |error: ValueError| SettingError::Invalid { key: key.name(), error };
                match key {
                    $($(SettingKey::$variant => {
                        let v: $ty = FromJson::from_json(value).map_err(invalid)?;
                        if v.is_unset() {
                            self.$field = None;
                            return Ok(serde_json::Value::Null);
                        }
                        let out = serde_json::to_value(&v).unwrap_or(serde_json::Value::Null);
                        self.$field = Some(v);
                        Ok(out)
                    })*)*
                    SettingKey::LabelEnabled(l) => {
                        let v: bool = FromJson::from_json(value).map_err(invalid)?;
                        self.labels.entry(l).or_default().enabled = Some(v);
                        Ok(serde_json::Value::Bool(v))
                    }
                    SettingKey::LabelThreshold(l) => {
                        // Empty: the general threshold applies.
                        let Some(v) = Option::<Probability>::from_json(value).map_err(invalid)? else {
                            if let Some(x) = self.labels.get_mut(&l) {
                                x.threshold = None;
                            }
                            self.labels.retain(|_, x| x.enabled.is_some() || x.threshold.is_some());
                            return Ok(serde_json::Value::Null);
                        };
                        self.labels.entry(l).or_default().threshold = Some(v);
                        Ok(serde_json::to_value(v).unwrap_or(serde_json::Value::Null))
                    }
                }
            }

            /// Removes `key` from this layer; returns whether it was set.
            pub fn clear(&mut self, key: SettingKey) -> bool {
                let was = match key {
                    $($(SettingKey::$variant => self.$field.take().is_some(),)*)*
                    SettingKey::LabelEnabled(l) => self.labels.get_mut(&l).and_then(|x| x.enabled.take()).is_some(),
                    SettingKey::LabelThreshold(l) => self.labels.get_mut(&l).and_then(|x| x.threshold.take()).is_some(),
                };
                self.labels.retain(|_, x| x.enabled.is_some() || x.threshold.is_some());
                was
            }

            /// Keys set in this layer.
            pub fn keys(&self) -> Vec<SettingKey> {
                SettingKey::all().into_iter().filter(|k| self.get_json(*k).is_some()).collect()
            }

            /// Keys set here that may not be set at `scope`.
            pub fn misplaced(&self, scope: ScopeKind) -> Vec<SettingKey> {
                self.keys().into_iter().filter(|k| !k.scopes().contains(&scope)).collect()
            }
        }

        /// Built-in defaults.
        #[derive(Debug, Clone, PartialEq, Serialize)]
        pub struct Defaults {
            $($(pub $field: $ty,)*)*
        }

        impl Defaults {
            /// The built-in values (made once).
            pub fn builtin() -> &'static Defaults {
                static BUILTIN: std::sync::LazyLock<Defaults> =
                    std::sync::LazyLock::new(|| Defaults { $($($field: $default,)*)* });
                &BUILTIN
            }
        }

        /// Every setting resolved for one place (global, a community, or a person in a community).
        #[derive(Debug, Clone, PartialEq, Serialize)]
        pub struct Effective {
            $($(pub $field: Resolved<$ty>,)*)*
            /// Per detection type: enabled and threshold (the threshold already falls back to the general one).
            pub labels: BTreeMap<Label, ResolvedLabel>,
        }

        /// Resolves every setting from the layers (most specific first).
        pub fn resolve(layers: &Layers<'_>) -> Effective {
            let d = Defaults::builtin();
            let order = layers.ordered();
            Effective {
                $($($field: {
                    let mut found = None;
                    for (source, layer) in &order {
                        if let Some(v) = &layer.$field {
                            found = Some(Resolved { value: v.clone(), source: *source });
                            break;
                        }
                    }
                    found.unwrap_or(Resolved { value: d.$field.clone(), source: Source::Default })
                },)*)*
                labels: resolve_labels(&order),
            }
        }
    };
}

fn lang(s: &str) -> Lang {
    s.parse().unwrap_or_else(|_| unreachable!("built-in language tag {s}"))
}

settings! {
    Tracking {
        paused(Paused): bool = false; ALL_SCOPES, Admins, Live;
        guild_allowlist(GuildAllowlist): Vec<GuildId> = Vec::new(); G, Owner, Live;
        tracked_everywhere(TrackedEverywhere): Vec<UserId> = Vec::new(); G, Owner, Live;
        allow_e2ee_downgrade(AllowE2eeDowngrade): bool = true; GS, Admins, Live;
        join_settle(JoinSettle): Dur = Dur::from_millis(1500); G, Owner, Live;
        leave_grace(LeaveGrace): Dur = Dur::from_millis(5000); G, Owner, Live;
    }
    Detection {
        threshold(Threshold): Probability = Probability::new(0.3).unwrap_or_else(|_| unreachable!()); ALL_SCOPES, Admins, Live;
        strikes(Strikes): Count = Count::new(1).unwrap_or_else(|_| unreachable!()); ALL_SCOPES, Admins, Live;
        strike_window(StrikeWindow): Limit<PosDur> = Limit::Unlimited; ALL_SCOPES, Admins, Live;
        end_silence(EndSilence): FrameDur = FrameDur::from_millis(600); ALL_SCOPES, Admins, Live;
        max_sentence(MaxSentence): PosDur = PosDur::from_millis(10_000); ALL_SCOPES, Admins, Live;
        min_voiced(MinVoiced): FrameDur = FrameDur::from_millis(300); ALL_SCOPES, Admins, Live;
        max_reaction_delay(MaxReactionDelay): Limit<PosDur> = Limit::Value(PosDur::from_millis(15_000)); ALL_SCOPES, Admins, Live;
    }
    Warning {
        observe_only(ObserveOnly): bool = false; ALL_SCOPES, Admins, Live;
        audience(Audience): AudienceChoice = AudienceChoice::Channel; ALL_SCOPES, Admins, Live;
        volume_db(VolumeDb): Finite = Finite::new(0.0).unwrap_or_else(|_| unreachable!()); ALL_SCOPES, Admins, Live;
        voice_language(VoiceLanguage): VoiceLang = VoiceLang::Fixed(lang("de")); ALL_SCOPES, Admins, Live;
        fallback_languages(FallbackLanguages): Vec<Lang> = vec![lang("en")]; ALL_SCOPES, Admins, Live;
        tts_voices(TtsVoices): BTreeMap<Lang, String> = BTreeMap::from([(lang("de"), "de_DE-thorsten-medium".to_owned())]); ALL_SCOPES, Admins, Live;
        line_voices(LineVoices): BTreeMap<LineKind, String> = BTreeMap::new(); ALL_SCOPES, Admins, Live;
        speech_rate(SpeechRate): Rate = Rate::new(1.1).unwrap_or_else(|_| unreachable!()); ALL_SCOPES, Admins, Live;
        no_speak_policy(NoSpeakPolicy): NoSpeakPolicy = NoSpeakPolicy::Text; ALL_SCOPES, Admins, Live;
        strike_notice(StrikeNotice): bool = false; ALL_SCOPES, Admins, Live;
        announce_actions(AnnounceActions): bool = true; ALL_SCOPES, Admins, Live;
        greet_enabled(GreetEnabled): bool = true; ALL_SCOPES, Admins, Live;
    }
    Escalation {
        violation_window(ViolationWindow): Limit<PosDur> = Limit::Value(PosDur::from_millis(3_600_000)); ALL_SCOPES, Admins, Live;
        escalation(Escalation): Escalation = Escalation::default_steps(); ALL_SCOPES, Admins, Live;
        actions_enabled(ActionsEnabled): bool = false; ALL_SCOPES, Admins, Live;
    }
    Reporting {
        modlog_channel(ModlogChannel): Option<ChannelId> = None; S, Admins, Live;
        modlog_audio(ModlogAudio): bool = true; GS, Owner, Live;
        owner_dm_audio(OwnerDmAudio): bool = false; ALL_SCOPES, Owner, Live;
        digest(Digest): Digest = Digest::Off; G, Owner, Live;
        digest_time(DigestTime): TimeOfDay = TimeOfDay { hour: 9, minute: 0 }; G, Owner, Live;
        digest_weekday(DigestWeekday): Weekday = Weekday::Monday; G, Owner, Live;
        timezone(Timezone): Tz = "UTC".parse().unwrap_or_else(|_| unreachable!()); G, Owner, Live;
        jar_enabled(JarEnabled): bool = true; ALL_SCOPES, Admins, Live;
        chat_language(ChatLanguage): Lang = lang("en"); GS, Admins, Live;
    }
    Recording {
        recordings(Recordings): Recordings = Recordings::Flagged; GS, Owner, Live;
        admins_play_audio(AdminsPlayAudio): bool = false; G, Owner, Live;
    }
    Commands {
        commands_enabled(CommandsEnabled): bool = true; G, Owner, Live;
        command_prefix(CommandPrefix): Prefix = "!pb".parse().unwrap_or_else(|_| unreachable!()); G, Owner, Live;
        admin_user_ids(AdminUserIds): Vec<UserId> = Vec::new(); G, Owner, Live;
        admin_role_ids(AdminRoleIds): Vec<RoleId> = Vec::new(); GS, Admins, Live;
    }
    System {
        instance(Instance): InstanceUrl = "https://api.fluxer.app".parse().unwrap_or_else(|_| unreachable!()); G, Owner, Reconnect;
        ui_url(UiUrl): Option<Origin> = None; G, Owner, Live;
        allowed_hosts(AllowedHosts): Vec<HostName> = Vec::new(); G, Owner, Live;
        cpu_threads(CpuThreads): Count = Count::new(4).unwrap_or_else(|_| unreachable!()); G, Owner, Live;
        tts_threads(TtsThreads): Count = Count::new(2).unwrap_or_else(|_| unreachable!()); G, Owner, Live;
    }
}

/// The layers for one place, most specific first.
#[derive(Debug, Clone, Copy, Default)]
pub struct Layers<'a> {
    pub person: Option<&'a Layer>,
    pub server: Option<&'a Layer>,
    pub global: Option<&'a Layer>,
    pub file: Option<&'a Layer>,
}

impl<'a> Layers<'a> {
    pub(crate) fn ordered(&self) -> Vec<(Source, &'a Layer)> {
        [
            (Source::Person, self.person),
            (Source::Server, self.server),
            (Source::Global, self.global),
            (Source::File, self.file),
        ]
        .into_iter()
        .filter_map(|(s, l)| l.map(|l| (s, l)))
        .collect()
    }
}

/// A detection type resolved.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResolvedLabel {
    pub enabled: Resolved<bool>,
    /// The bar a score must reach: in the most specific layer that sets either, the detection type's own threshold
    /// beats the general one.
    pub threshold: Resolved<f64>,
}

/// The detection types that are on unless a setting says otherwise: all of them.
fn enabled_by_default(_label: Label) -> bool {
    true
}

fn resolve_labels(order: &[(Source, &Layer)]) -> BTreeMap<Label, ResolvedLabel> {
    let default_threshold = Defaults::builtin().threshold.get();
    Label::ALL
        .into_iter()
        .map(|label| {
            let enabled = order
                .iter()
                .find_map(|(s, l)| {
                    l.labels
                        .get(&label)
                        .and_then(|x| x.enabled)
                        .map(|v| Resolved { value: v, source: *s })
                })
                .unwrap_or(Resolved {
                    value: enabled_by_default(label),
                    source: Source::Default,
                });
            let threshold = order
                .iter()
                .find_map(|(s, l)| {
                    l.labels
                        .get(&label)
                        .and_then(|x| x.threshold)
                        .or(l.threshold)
                        .map(|v| Resolved {
                            value: v.get(),
                            source: *s,
                        })
                })
                .unwrap_or(Resolved {
                    value: default_threshold,
                    source: Source::Default,
                });
            (label, ResolvedLabel { enabled, threshold })
        })
        .collect()
}

impl Effective {
    /// Whether recordings of this community may be played: by the owner always, by its admins when allowed.
    pub fn may_play_recordings(&self, owner: bool, admin_here: bool) -> bool {
        owner || (admin_here && self.admins_play_audio.value)
    }

    /// Detection types the bot reacts to here.
    pub fn enabled_labels(&self) -> Vec<Label> {
        self.labels
            .iter()
            .filter(|(_, r)| r.enabled.value)
            .map(|(l, _)| *l)
            .collect()
    }

    /// The bar for `label`.
    pub fn threshold_for(&self, label: Label) -> f64 {
        self.labels
            .get(&label)
            .map_or(self.threshold.value.get(), |r| r.threshold.value)
    }
}

/// A value typed as text (in chat or a web form) as JSON for the validator: lists split at commas and spaces,
/// `lang=voice` pairs as a map, mentions like `<@1>`, `<@&2>` and `<#3>` (as pasted from Fluxer) as their ids, and
/// everything else as the text it is (the validator reads switches, numbers and durations from text).
pub fn text_value(key: SettingKey, raw: &str) -> serde_json::Value {
    use serde_json::Value;
    let raw = raw.trim();
    let unmention = |s: &str| Value::String(pb_domain::unmention(s).to_owned());
    match key.meta().kind {
        // `de=thorsten-high en=lessac-medium`, `warning=piper:de_DE-thorsten-high`
        FieldKind::Voices | FieldKind::LineVoices => Value::Object(
            raw.split([',', ' ', '\n'])
                .filter_map(|p| p.split_once(['=', ':']))
                .map(|(l, v)| (l.trim().to_owned(), Value::String(v.trim().to_owned())))
                .filter(|(l, v)| !l.is_empty() && v.as_str().is_some_and(|v| !v.is_empty()))
                .collect(),
        ),
        FieldKind::Ids { .. } | FieldKind::Hosts | FieldKind::Langs => Value::Array(
            raw.split([',', ' ', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(unmention)
                .collect(),
        ),
        FieldKind::Channel => unmention(raw),
        // Switches, numbers, durations and choices: the validator reads them from text.
        _ => Value::String(raw.into()),
    }
}

/// Validates and sets `key` at `scope` in `layer`, for someone who is (or is not) the bot owner.
pub fn set(
    layer: &mut Layer,
    scope: ScopeKind,
    key: SettingKey,
    value: serde_json::Value,
    by_owner: bool,
) -> Result<serde_json::Value, SettingError> {
    if !key.scopes().contains(&scope) {
        return Err(SettingError::Scope { key: key.name(), scope });
    }
    if key.who() == Who::Owner && !by_owner {
        return Err(SettingError::OwnerOnly { key: key.name() });
    }
    layer.set_json(key, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_round_trips_and_has_metadata() {
        let all = SettingKey::all();
        assert_eq!(all.len(), all.iter().collect::<std::collections::BTreeSet<_>>().len());
        for k in all {
            assert_eq!(k.name().parse::<SettingKey>(), Ok(k));
            let m = k.meta();
            assert!(!m.scopes.is_empty());
        }
    }

    #[test]
    fn person_beats_server_beats_global_beats_file_beats_default() {
        let mut person = Layer::default();
        let mut server = Layer::default();
        let mut global = Layer::default();
        let mut file = Layer::default();
        file.set_json(SettingKey::Strikes, serde_json::json!(5)).expect("set");
        let e = resolve(&Layers {
            file: Some(&file),
            ..Layers::default()
        });
        assert_eq!((e.strikes.value.get(), e.strikes.source), (5, Source::File));
        global.set_json(SettingKey::Strikes, serde_json::json!(4)).expect("set");
        server.set_json(SettingKey::Strikes, serde_json::json!(3)).expect("set");
        person.set_json(SettingKey::Strikes, serde_json::json!(2)).expect("set");
        let e = resolve(&Layers {
            person: Some(&person),
            server: Some(&server),
            global: Some(&global),
            file: Some(&file),
        });
        assert_eq!((e.strikes.value.get(), e.strikes.source), (2, Source::Person));
        let e = resolve(&Layers::default());
        assert_eq!((e.strikes.value.get(), e.strikes.source), (1, Source::Default));
    }

    #[test]
    fn a_label_threshold_beats_the_general_one_in_the_same_layer_only() {
        let mut server = Layer::default();
        let mut person = Layer::default();
        server
            .set_json(SettingKey::LabelThreshold(Label::Profanity), serde_json::json!(0.9))
            .expect("set");
        person
            .set_json(SettingKey::Threshold, serde_json::json!(0.25))
            .expect("set");
        let e = resolve(&Layers {
            person: Some(&person),
            server: Some(&server),
            ..Layers::default()
        });
        assert_eq!(
            e.threshold_for(Label::Profanity),
            0.25,
            "the person's general threshold is more specific"
        );
        let e = resolve(&Layers {
            server: Some(&server),
            ..Layers::default()
        });
        assert_eq!(e.threshold_for(Label::Profanity), 0.9);
        assert_eq!(e.threshold_for(Label::Harassment), 0.3);
        assert_eq!(e.enabled_labels(), Label::ALL.to_vec());
    }

    #[test]
    fn scopes_and_owner_rules_are_enforced() {
        let mut l = Layer::default();
        assert!(matches!(
            set(
                &mut l,
                ScopeKind::Global,
                SettingKey::ModlogChannel,
                serde_json::json!("1"),
                true
            ),
            Err(SettingError::Scope { .. })
        ));
        assert!(matches!(
            set(
                &mut l,
                ScopeKind::Global,
                SettingKey::Digest,
                serde_json::json!("daily"),
                false
            ),
            Err(SettingError::OwnerOnly { .. })
        ));
        assert!(
            set(
                &mut l,
                ScopeKind::Server,
                SettingKey::ModlogChannel,
                serde_json::json!("123"),
                false
            )
            .is_ok()
        );
        assert!(matches!(
            set(
                &mut l,
                ScopeKind::Server,
                SettingKey::Threshold,
                serde_json::json!(1.5),
                false
            ),
            Err(SettingError::Invalid { .. })
        ));
        assert_eq!(l.misplaced(ScopeKind::Global), vec![SettingKey::ModlogChannel]);
    }

    #[test]
    fn line_voices_are_typed_as_pairs() {
        let mut l = Layer::default();
        let v = text_value(
            SettingKey::LineVoices,
            "warning=piper:de_DE-thorsten-high, greeting=x:y",
        );
        l.set_json(SettingKey::LineVoices, v).expect("set");
        let set = l.line_voices.clone().expect("set");
        assert_eq!(set[&LineKind::Warning], "piper:de_DE-thorsten-high");
        assert_eq!(set[&LineKind::Greeting], "x:y");
        let bad = text_value(SettingKey::LineVoices, "name=piper:x");
        assert!(l.set_json(SettingKey::LineVoices, bad).is_err());
        let toml: Layer = toml::from_str("[line_voices]\nstrike = \"piper:en_US-lessac-medium\"\n").expect("toml");
        assert_eq!(
            toml.line_voices.expect("set")[&LineKind::StrikeNotice],
            "piper:en_US-lessac-medium"
        );
    }

    #[test]
    fn layers_read_from_toml_and_reject_typos() {
        let l: Layer = toml::from_str("strikes = 2\nstrike_window = \"30s\"\n[labels.harassment]\nenabled = true\n")
            .expect("toml");
        assert_eq!(l.strikes.map(Count::get), Some(2));
        assert!(toml::from_str::<Layer>("strikez = 2").is_err());
        assert!(toml::from_str::<Layer>("threshold = 2.0").is_err());
    }
}
