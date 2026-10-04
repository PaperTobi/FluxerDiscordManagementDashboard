//! Placeholders in voice-line texts: `{name} {label} {count} {step} {strikes} {duration} {server} {channel}`.
//! Values are inserted literally (never parsed again); unknown placeholders stay as written.

use std::collections::BTreeMap;

/// A placeholder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Field {
    /// The person's spoken name.
    Name,
    /// The detection type, spoken.
    Label,
    /// How many violations in the counting window (including this one).
    Count,
    /// The escalation step.
    Step,
    /// Strikes needed before a warning.
    Strikes,
    /// How long a moderation action lasts, as words ("5 minutes").
    Duration,
    /// The community's name.
    Server,
    /// The voice channel's name.
    Channel,
}

impl Field {
    pub const ALL: [Field; 8] = [
        Field::Name,
        Field::Label,
        Field::Count,
        Field::Step,
        Field::Strikes,
        Field::Duration,
        Field::Server,
        Field::Channel,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Field::Name => "name",
            Field::Label => "label",
            Field::Count => "count",
            Field::Step => "step",
            Field::Strikes => "strikes",
            Field::Duration => "duration",
            Field::Server => "server",
            Field::Channel => "channel",
        }
    }
}

/// Values for placeholders.
pub type Fields = BTreeMap<Field, String>;

/// Fills placeholders; a placeholder without a value, or an unknown one, stays as written.
pub fn fill(text: &str, fields: &Fields) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let name = &after[..close];
                match Field::ALL
                    .into_iter()
                    .find(|f| f.as_str() == name)
                    .and_then(|f| fields.get(&f))
                {
                    Some(value) => out.push_str(value),
                    None => {
                        out.push('{');
                        out.push_str(name);
                        out.push('}');
                    }
                }
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_literally_and_keeps_unknowns() {
        let mut f = Fields::new();
        f.insert(Field::Name, "{step} Max".into());
        f.insert(Field::Step, "2".into());
        assert_eq!(
            fill("Hey {name}, step {step}, {nope} {name.__class__} {", &f),
            "Hey {step} Max, step 2, {nope} {name.__class__} {"
        );
    }
}
