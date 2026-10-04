use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use pb_domain::{ActionKind, BlobHash, Label, Lang};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A specific value or "any" (matches every value; less specific than a value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sel<T> {
    Any,
    Is(T),
}

/// What is being said.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Line {
    /// The warning after a violation: per detection type and escalation step.
    Warning { label: Sel<Label>, step: Sel<u32> },
    /// Said when the bot and a tracked person meet in a call.
    Greeting,
    /// Said for a flagged sentence that does not yet reach the strike count.
    StrikeNotice,
    /// Said when a moderation action is taken.
    Action { kind: Sel<ActionKind> },
    /// A ready-made "say now" text or clip, by name.
    Say { preset: String },
    /// How the person's name is said (spliced into other lines at `{name}`).
    Name,
}

/// The stable text form of a [`Line`], used as a key in settings files and URLs:
/// `warning.<label|any>.<step|any>`, `greeting`, `strike`, `action.<kind|any>`, `say.<preset>`, `name`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineKey(pub Line);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("not a voice line: {0:?}")]
pub struct LineKeyError(pub String);

fn sel_str<T: fmt::Display>(s: &Sel<T>) -> String {
    match s {
        Sel::Any => "any".into(),
        Sel::Is(v) => v.to_string(),
    }
}

impl fmt::Display for LineKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            Line::Warning { label, step } => write!(f, "warning.{}.{}", sel_str(label), sel_str(step)),
            Line::Greeting => f.write_str("greeting"),
            Line::StrikeNotice => f.write_str("strike"),
            Line::Action { kind } => match kind {
                Sel::Any => f.write_str("action.any"),
                Sel::Is(k) => write!(f, "action.{}", action_str(*k)),
            },
            Line::Say { preset } => write!(f, "say.{preset}"),
            Line::Name => f.write_str("name"),
        }
    }
}

fn action_str(k: ActionKind) -> &'static str {
    match k {
        ActionKind::Mute => "mute",
        ActionKind::Unmute => "unmute",
        ActionKind::Disconnect => "disconnect",
        ActionKind::Timeout => "timeout",
    }
}

fn valid_preset(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

impl FromStr for LineKey {
    type Err = LineKeyError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || LineKeyError(s.to_owned());
        let parts: Vec<&str> = s.split('.').collect();
        let line = match parts.as_slice() {
            ["greeting"] => Line::Greeting,
            ["strike"] => Line::StrikeNotice,
            ["name"] => Line::Name,
            ["warning", label, step] => {
                let label = if *label == "any" {
                    Sel::Any
                } else {
                    Sel::Is(label.parse::<Label>().map_err(|_| err())?)
                };
                let step = if *step == "any" {
                    Sel::Any
                } else {
                    let n: u32 = step.parse().map_err(|_| err())?;
                    if n == 0 {
                        return Err(err());
                    }
                    Sel::Is(n)
                };
                Line::Warning { label, step }
            }
            ["action", kind] => Line::Action {
                kind: match *kind {
                    "any" => Sel::Any,
                    "mute" => Sel::Is(ActionKind::Mute),
                    "unmute" => Sel::Is(ActionKind::Unmute),
                    "disconnect" => Sel::Is(ActionKind::Disconnect),
                    "timeout" => Sel::Is(ActionKind::Timeout),
                    _ => return Err(err()),
                },
            },
            ["say", preset] if valid_preset(preset) => Line::Say {
                preset: (*preset).to_owned(),
            },
            _ => return Err(err()),
        };
        Ok(LineKey(line))
    }
}

impl Serialize for LineKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for LineKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// One voice line at one scope: clips (one is picked; each has a language in the clip library) and text per language.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Slot {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<BlobHash>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub text: BTreeMap<Lang, String>,
}

impl Slot {
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty() && self.text.values().all(|t| t.trim().is_empty())
    }
}

/// All voice lines set at one scope.
pub type Slots = BTreeMap<LineKey, Slot>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_keys_round_trip() {
        for k in [
            "warning.profanity.2",
            "warning.any.any",
            "warning.harassment.any",
            "warning.any.3",
            "greeting",
            "strike",
            "action.mute",
            "action.any",
            "say.calm-down",
            "name",
        ] {
            let parsed: LineKey = k.parse().expect(k);
            assert_eq!(parsed.to_string(), k);
        }
        for bad in [
            "warning.profanity.0",
            "warning.cursing.1",
            "say.",
            "say.a b",
            "action.ban",
            "hello",
            "warning.any",
        ] {
            assert!(bad.parse::<LineKey>().is_err(), "{bad}");
        }
    }

    #[test]
    fn slots_read_from_toml() {
        let text = r#"
            ["warning.profanity.2"]
            clips = ["sha256:abababababababababababababababababababababababababababababababab"]
            text.de = "{name}, das ist schon das zweite Mal."
            text.en = "{name}, that's twice now."
        "#;
        let slots: Slots = toml::from_str(text).expect("parses");
        let slot = &slots[&"warning.profanity.2".parse::<LineKey>().expect("key")];
        assert_eq!(slot.clips.len(), 1);
        assert_eq!(
            slot.text[&"de".parse::<Lang>().expect("de")],
            "{name}, das ist schon das zweite Mal."
        );
    }
}
