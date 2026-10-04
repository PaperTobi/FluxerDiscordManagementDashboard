//! The texts the bot ships with, per language. They are the last scope in the resolution (after person, server and
//! global); people replace them with their own texts or clips.

use pb_domain::Lang;

use super::line::{Line, Sel};

/// The built-in text for a line in a language (by base language), if there is one.
pub fn builtin_text(line: &Line, lang: &Lang) -> Option<&'static str> {
    let de = lang.language() == "de";
    let en = lang.language() == "en";
    if !de && !en {
        return None;
    }
    let pick = |e: &'static str, d: &'static str| Some(if de { d } else { e });
    match line {
        Line::Warning {
            label: Sel::Any,
            step: Sel::Is(1),
        } => pick(
            "Hey {name}, watch your language!",
            "Hey {name}, achte auf deine Wortwahl!",
        ),
        Line::Warning {
            label: Sel::Any,
            step: Sel::Is(2),
        } => pick(
            "{name}, that's twice now. Keep it clean.",
            "{name}, das ist schon das zweite Mal. Bitte reiß dich zusammen.",
        ),
        Line::Warning {
            label: Sel::Any,
            step: Sel::Is(3),
        } => pick(
            "{name}, last warning. Cut it out.",
            "{name}, letzte Warnung. Hör auf damit.",
        ),
        Line::Greeting => pick("Hi {name}, I'm listening.", "Hallo {name}, ich höre zu."),
        Line::StrikeNotice => pick(
            "{name}, careful. That's {count} of {strikes}.",
            "{name}, Vorsicht. Das ist {count} von {strikes}.",
        ),
        Line::Action { kind: Sel::Is(k) } => match k {
            pb_domain::ActionKind::Mute => pick(
                "{name} has been muted for {duration}.",
                "{name} wurde für {duration} stummgeschaltet.",
            ),
            pb_domain::ActionKind::Unmute => pick(
                "{name}, you're unmuted again.",
                "{name}, du bist wieder freigeschaltet.",
            ),
            pb_domain::ActionKind::Disconnect => pick(
                "{name} was disconnected from the call.",
                "{name} wurde aus dem Anruf entfernt.",
            ),
            pb_domain::ActionKind::Timeout => pick(
                "{name} is in timeout for {duration}.",
                "{name} hat eine Auszeit für {duration}.",
            ),
        },
        _ => None,
    }
}
