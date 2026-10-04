//! A resolution plus the situation → exactly what to play: clips, texts to speak (placeholders filled) and pauses.

use pb_domain::{BlobHash, Lang};

use super::pick::pick;
use super::resolve::Resolution;
use super::template::{Fields, fill};

/// One piece of an utterance.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Part {
    /// Speak this text in this language (with the voice chosen for that language).
    Speak {
        lang: Lang,
        text: String,
    },
    Clip(BlobHash),
    Silence {
        ms: u32,
    },
}

/// What to play, in order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct UtterancePlan {
    pub parts: Vec<Part>,
}

impl UtterancePlan {
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

/// Pause around a spliced name clip.
const NAME_GAP_MS: u32 = 120;

/// Builds the plan. `name_clip`: the person's recorded name in the resolution's language (spliced in at `{name}`);
/// otherwise `{name}` is filled from `fields`. `last`: the clip played last for this person and line (not repeated);
/// `random`: a number in `0..n`.
pub fn plan(
    res: &Resolution,
    fields: &Fields,
    name_clip: Option<&BlobHash>,
    last: Option<&BlobHash>,
    random: &mut dyn FnMut(usize) -> usize,
) -> UtterancePlan {
    match res {
        Resolution::Silent => UtterancePlan::default(),
        Resolution::Clips { clips, .. } => UtterancePlan {
            parts: pick(clips, last, random)
                .map(|c| vec![Part::Clip(*c)])
                .unwrap_or_default(),
        },
        Resolution::Text { lang, text, .. } => {
            let Some(name) = name_clip else {
                let filled = fill(text, fields);
                return UtterancePlan {
                    parts: vec![Part::Speak {
                        lang: lang.clone(),
                        text: filled,
                    }],
                };
            };
            // The text around each `{name}` is spoken when it has words (not only punctuation).
            let mut pieces = Vec::new();
            for (i, piece) in text.split("{name}").enumerate() {
                if i > 0 {
                    pieces.push(Part::Clip(*name));
                }
                let filled = fill(piece, fields);
                let words = if i > 0 {
                    filled.trim_start_matches([',', ' ', ';', ':'])
                } else {
                    &filled
                }
                .trim();
                if words.chars().any(char::is_alphanumeric) {
                    pieces.push(Part::Speak {
                        lang: lang.clone(),
                        text: words.to_owned(),
                    });
                }
            }
            // A short pause wherever a recorded name meets speech or another name.
            let mut parts = Vec::with_capacity(pieces.len() * 2);
            for p in pieces {
                if !parts.is_empty() {
                    parts.push(Part::Silence { ms: NAME_GAP_MS });
                }
                parts.push(p);
            }
            UtterancePlan { parts }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Field, LineKey, Source};

    fn res(text: &str) -> Resolution {
        Resolution::Text {
            source: Source::Builtin,
            key: "greeting".parse::<LineKey>().expect("key"),
            lang: "en".parse().expect("en"),
            text: text.into(),
        }
    }

    #[test]
    fn fills_the_name_as_text_without_a_clip() {
        let fields = Fields::from([(Field::Name, "Richard".to_owned())]);
        let p = plan(&res("Hi {name}, I'm listening."), &fields, None, None, &mut |_| 0);
        assert_eq!(
            p.parts,
            vec![Part::Speak {
                lang: "en".parse().expect("en"),
                text: "Hi Richard, I'm listening.".into()
            }]
        );
    }

    #[test]
    fn splices_a_recorded_name() {
        let name = BlobHash::from_bytes([7; 32]);
        let p = plan(
            &res("Hey {name}, watch your language!"),
            &Fields::new(),
            Some(&name),
            None,
            &mut |_| 0,
        );
        let en: Lang = "en".parse().expect("en");
        assert_eq!(
            p.parts,
            vec![
                Part::Speak {
                    lang: en.clone(),
                    text: "Hey".into()
                },
                Part::Silence { ms: 120 },
                Part::Clip(name),
                Part::Silence { ms: 120 },
                Part::Speak {
                    lang: en,
                    text: "watch your language!".into()
                },
            ]
        );
        let p = plan(&res("{name}"), &Fields::new(), Some(&name), None, &mut |_| 0);
        assert_eq!(p.parts, vec![Part::Clip(name)]);
        // Punctuation alone after the name is not spoken (and adds no pause).
        let p = plan(&res("Hey {name}!"), &Fields::new(), Some(&name), None, &mut |_| 0);
        assert_eq!(
            p.parts,
            vec![
                Part::Speak {
                    lang: "en".parse().expect("en"),
                    text: "Hey".into()
                },
                Part::Silence { ms: 120 },
                Part::Clip(name),
            ]
        );
    }
}
