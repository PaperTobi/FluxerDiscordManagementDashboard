//! Which slot speaks a line, in which language, with a clip or a text.
//!
//! Order (see the crate docs of `pb-voicelines` and docs/design.md §7):
//! - languages in preference order (the person's language or the one they just spoke, then the fallbacks);
//! - for each language, the slots from most to least specific: the exact escalation step at person, server and global
//!   scope; then "any step" slots there; then the built-in exact step; then lower steps, each at person, server,
//!   global and built-in (so a person's own step-1 text never replaces the built-in step-3 text); inside a scope the
//!   specific detection type before "any";
//! - in a slot, a clip in that language (or one without speech) wins; else its text, if a voice speaks that language;
//! - if nothing speaks, the shipped fallback clips (when the line has them).

use pb_domain::{BlobHash, Lang};

use super::builtin::builtin_text;
use super::line::{Line, LineKey, Sel, Slot, Slots};

/// What the clip library knows about a clip's language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipLang {
    Speech(Lang),
    /// A sound without words: fits every language.
    NonSpeech,
    /// Not tagged yet: used only when nothing tagged fits.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipInfo {
    pub lang: ClipLang,
}

/// Where a resolution came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Person,
    Server,
    Global,
    Builtin,
    /// The shipped clips, used when no voice speaks any of the languages.
    Fallback,
}

/// The slots of the three scopes (missing scopes are `None`).
#[derive(Debug, Clone, Copy, Default)]
pub struct ScopedSlots<'a> {
    pub person: Option<&'a Slots>,
    pub server: Option<&'a Slots>,
    pub global: Option<&'a Slots>,
}

impl ScopedSlots<'_> {
    /// The "say" presets any of the scopes has (for "Say now").
    pub fn say_presets(&self) -> std::collections::BTreeSet<String> {
        [self.person, self.server, self.global]
            .into_iter()
            .flatten()
            .flat_map(|s| s.keys())
            .filter_map(|k| match &k.0 {
                crate::Line::Say { preset } => Some(preset.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Everything resolution needs besides the line.
pub struct ResolveCtx<'a> {
    /// Languages in preference order (at least one).
    pub languages: &'a [Lang],
    pub slots: ScopedSlots<'a>,
    /// Looks a clip up in the library (`None` = not in the library any more).
    pub clip: &'a dyn Fn(&BlobHash) -> Option<ClipInfo>,
    /// Whether a text-to-speech voice speaks this language.
    pub has_voice: &'a dyn Fn(&Lang) -> bool,
    /// Shipped clips for this line, used only when nothing else can speak.
    pub fallback_clips: &'a [BlobHash],
}

impl std::fmt::Debug for ResolveCtx<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolveCtx")
            .field("languages", &self.languages)
            .finish_non_exhaustive()
    }
}

/// The outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// Play one of these clips.
    Clips {
        source: Source,
        key: LineKey,
        lang: Option<Lang>,
        clips: Vec<BlobHash>,
    },
    /// Speak this text (placeholders not filled yet) in this language.
    Text {
        source: Source,
        key: LineKey,
        lang: Lang,
        text: String,
    },
    /// Nothing can be said.
    Silent,
}

fn same_language(want: &Lang, have: &Lang) -> bool {
    have == want || have.language() == want.language()
}

/// Lines to try, most specific first, each with the scopes it is looked up in.
fn candidates(line: &Line) -> Vec<(Line, Stage)> {
    use Stage::*;
    match line {
        Line::Warning { label, step } => {
            let labels: Vec<Sel<pb_domain::Label>> = match label {
                Sel::Is(l) => vec![Sel::Is(*l), Sel::Any],
                Sel::Any => vec![Sel::Any],
            };
            let warn = |label: Sel<pb_domain::Label>, step: Sel<u32>| Line::Warning { label, step };
            let mut out = Vec::new();
            let exact = match step {
                Sel::Is(s) => *s,
                Sel::Any => 0,
            };
            if exact > 0 {
                for l in &labels {
                    out.push((warn(*l, Sel::Is(exact)), UserScopes));
                }
            }
            for l in &labels {
                out.push((warn(*l, Sel::Any), UserScopes));
            }
            if exact > 0 {
                out.push((warn(Sel::Any, Sel::Is(exact)), BuiltinOnly));
                for lower in (1..exact).rev() {
                    for l in &labels {
                        out.push((warn(*l, Sel::Is(lower)), AllScopes));
                    }
                    out.push((warn(Sel::Any, Sel::Is(lower)), BuiltinOnly));
                }
            }
            out
        }
        Line::Action { kind: Sel::Is(k) } => vec![
            (Line::Action { kind: Sel::Is(*k) }, AllScopes),
            (Line::Action { kind: Sel::Any }, UserScopes),
        ],
        Line::Name => vec![(Line::Name, PersonOnly)],
        other => vec![(other.clone(), AllScopes)],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    UserScopes,
    AllScopes,
    BuiltinOnly,
    PersonOnly,
}

fn try_slot(slot: &Slot, lang: &Lang, ctx: &ResolveCtx<'_>) -> Option<(Option<Lang>, Vec<BlobHash>, Option<String>)> {
    let infos: Vec<(BlobHash, ClipInfo)> = slot
        .clips
        .iter()
        .filter_map(|h| (ctx.clip)(h).map(|i| (*h, i)))
        .collect();
    let fitting: Vec<BlobHash> = infos
        .iter()
        .filter(|(_, i)| match &i.lang {
            ClipLang::Speech(l) => same_language(lang, l),
            ClipLang::NonSpeech => true,
            ClipLang::Unknown => false,
        })
        .map(|(h, _)| *h)
        .collect();
    if !fitting.is_empty() {
        return Some((Some(lang.clone()), fitting, None));
    }
    let text = slot
        .text
        .iter()
        .find(|(l, _)| *l == lang)
        .or_else(|| slot.text.iter().find(|(l, _)| same_language(lang, l)))
        .map(|(_, t)| t.trim())
        .filter(|t| !t.is_empty());
    if let Some(t) = text
        && (ctx.has_voice)(lang)
    {
        return Some((Some(lang.clone()), Vec::new(), Some(t.to_owned())));
    }
    None
}

/// Resolves a concrete line (`Warning` with a specific label and step, `Greeting`, …).
/// The scopes whose slots a candidate line is looked up in, most specific first.
fn scopes_for<'a>(stage: Stage, slots: &ScopedSlots<'a>) -> Vec<(Source, Option<&'a Slots>)> {
    let all = [
        (Source::Person, slots.person),
        (Source::Server, slots.server),
        (Source::Global, slots.global),
    ];
    let n = match stage {
        Stage::BuiltinOnly => 0,
        Stage::PersonOnly => 1,
        _ => all.len(),
    };
    all[..n].to_vec()
}

pub fn resolve(line: &Line, ctx: &ResolveCtx<'_>) -> Resolution {
    let cands = candidates(line);
    for lang in ctx.languages {
        for (cand, stage) in &cands {
            let key = LineKey(cand.clone());
            for (source, slots) in scopes_for(*stage, &ctx.slots) {
                if let Some(slot) = slots.and_then(|s| s.get(&key))
                    && let Some((l, clips, text)) = try_slot(slot, lang, ctx)
                {
                    return match text {
                        Some(text) => Resolution::Text {
                            source,
                            key,
                            lang: lang.clone(),
                            text,
                        },
                        None => Resolution::Clips {
                            source,
                            key,
                            lang: l,
                            clips,
                        },
                    };
                }
            }
            if matches!(stage, Stage::AllScopes | Stage::BuiltinOnly)
                && (ctx.has_voice)(lang)
                && let Some(text) = builtin_text(cand, lang)
            {
                return Resolution::Text {
                    source: Source::Builtin,
                    key,
                    lang: lang.clone(),
                    text: text.to_owned(),
                };
            }
        }
    }
    // Untagged clips, in the same slot order, before giving up.
    for (cand, stage) in &cands {
        let key = LineKey(cand.clone());
        for (source, slots) in scopes_for(*stage, &ctx.slots) {
            if let Some(slot) = slots.and_then(|s| s.get(&key)) {
                let untagged: Vec<BlobHash> = slot
                    .clips
                    .iter()
                    .filter(|h| {
                        matches!(
                            (ctx.clip)(h),
                            Some(ClipInfo {
                                lang: ClipLang::Unknown
                            })
                        )
                    })
                    .cloned()
                    .collect();
                if !untagged.is_empty() {
                    return Resolution::Clips {
                        source,
                        key,
                        lang: None,
                        clips: untagged,
                    };
                }
            }
        }
    }
    if !ctx.fallback_clips.is_empty() && !matches!(line, Line::Name) {
        return Resolution::Clips {
            source: Source::Fallback,
            key: LineKey(line.clone()),
            lang: None,
            clips: ctx.fallback_clips.to_vec(),
        };
    }
    Resolution::Silent
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use pb_domain::Label;

    use super::*;

    fn lang(s: &str) -> Lang {
        s.parse().expect("lang")
    }

    fn hash(n: u8) -> BlobHash {
        BlobHash::from_bytes([n; 32])
    }

    fn warning(step: u32) -> Line {
        Line::Warning {
            label: Sel::Is(Label::Profanity),
            step: Sel::Is(step),
        }
    }

    fn key(s: &str) -> LineKey {
        s.parse().expect("key")
    }

    fn text_slot(l: &str, t: &str) -> Slot {
        Slot {
            clips: vec![],
            text: BTreeMap::from([(lang(l), t.to_owned())]),
        }
    }

    fn resolve_with(
        line: &Line,
        langs: &[&str],
        person: &Slots,
        global: &Slots,
        voices: &[&str],
        clips: &[(u8, Option<&str>)],
    ) -> Resolution {
        let languages: Vec<Lang> = langs.iter().map(|l| lang(l)).collect();
        let voices: Vec<Lang> = voices.iter().map(|l| lang(l)).collect();
        let library: BTreeMap<BlobHash, ClipInfo> = clips
            .iter()
            .map(|(n, l)| {
                (
                    hash(*n),
                    ClipInfo {
                        lang: l.map_or(ClipLang::NonSpeech, |l| {
                            if l == "?" {
                                ClipLang::Unknown
                            } else {
                                ClipLang::Speech(lang(l))
                            }
                        }),
                    },
                )
            })
            .collect();
        let ctx = ResolveCtx {
            languages: &languages,
            slots: ScopedSlots {
                person: Some(person),
                server: None,
                global: Some(global),
            },
            clip: &|h| library.get(h).cloned(),
            has_voice: &|l| voices.iter().any(|v| v.language() == l.language()),
            fallback_clips: &[],
        };
        resolve(line, &ctx)
    }

    #[test]
    fn builtin_step_beats_a_persons_lower_step() {
        let person: Slots = [(key("warning.any.1"), text_slot("en", "my step one"))].into();
        let r = resolve_with(&warning(3), &["en"], &person, &Slots::new(), &["en"], &[]);
        assert!(
            matches!(r, Resolution::Text { source: Source::Builtin, ref text, .. } if text.contains("last warning")),
            "{r:?}"
        );
        let r = resolve_with(&warning(4), &["en"], &person, &Slots::new(), &["en"], &[]);
        assert!(
            matches!(r, Resolution::Text { source: Source::Builtin, ref text, .. } if text.contains("last warning")),
            "falls back to step 3: {r:?}"
        );
    }

    #[test]
    fn any_step_beats_builtin() {
        let person: Slots = [(key("warning.any.any"), text_slot("en", "mine"))].into();
        let r = resolve_with(&warning(2), &["en"], &person, &Slots::new(), &["en"], &[]);
        assert!(
            matches!(r, Resolution::Text { source: Source::Person, ref text, .. } if text == "mine"),
            "{r:?}"
        );
    }

    #[test]
    fn clips_in_the_language_win_inside_a_slot_and_specific_slots_win() {
        let global: Slots = [(
            key("warning.any.any"),
            Slot {
                clips: vec![hash(1)],
                text: BTreeMap::new(),
            },
        )]
        .into();
        let person: Slots = [(
            key("warning.profanity.any"),
            Slot {
                clips: vec![hash(2), hash(3)],
                text: BTreeMap::from([(lang("de"), "Text".into())]),
            },
        )]
        .into();
        let r = resolve_with(
            &warning(1),
            &["de"],
            &person,
            &global,
            &["de"],
            &[(1, Some("de")), (2, Some("en")), (3, Some("de"))],
        );
        assert_eq!(
            r,
            Resolution::Clips {
                source: Source::Person,
                key: key("warning.profanity.any"),
                lang: Some(lang("de")),
                clips: vec![hash(3)]
            }
        );
    }

    #[test]
    fn falls_back_through_languages_and_needs_a_voice_for_text() {
        let person: Slots = [(key("greeting"), text_slot("fr", "Salut {name}"))].into();
        let r = resolve_with(&Line::Greeting, &["fr", "de"], &person, &Slots::new(), &["de"], &[]);
        assert!(
            matches!(r, Resolution::Text { source: Source::Builtin, ref lang, .. } if lang.language() == "de"),
            "{r:?}"
        );
        let r = resolve_with(&Line::Greeting, &["fr"], &person, &Slots::new(), &[], &[]);
        assert_eq!(r, Resolution::Silent);
    }

    #[test]
    fn non_speech_clips_fit_every_language_and_untagged_ones_come_last() {
        let person: Slots = [(
            key("greeting"),
            Slot {
                clips: vec![hash(9)],
                text: BTreeMap::new(),
            },
        )]
        .into();
        let r = resolve_with(&Line::Greeting, &["de"], &person, &Slots::new(), &[], &[(9, None)]);
        assert!(
            matches!(
                r,
                Resolution::Clips {
                    source: Source::Person,
                    ..
                }
            ),
            "{r:?}"
        );
        let r = resolve_with(&Line::Greeting, &["de"], &person, &Slots::new(), &[], &[(9, Some("?"))]);
        assert!(matches!(r, Resolution::Clips { lang: None, .. }), "{r:?}");
    }
}
