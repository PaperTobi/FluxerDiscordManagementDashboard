//! What the Roblox voice-safety classifier reports: eight abuse labels and thirty language heads.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// The classifier's abuse labels, in the model's output order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    PrivacyAskingForPii,
    Discriminatory,
    Harassment,
    SexualContent,
    IllegalAndRegulatedContent,
    DatingAndRomanticContent,
    Profanity,
    DisruptiveAudio,
}

impl Label {
    /// All labels in the model's output order.
    pub const ALL: [Label; 8] = [
        Label::PrivacyAskingForPii,
        Label::Discriminatory,
        Label::Harassment,
        Label::SexualContent,
        Label::IllegalAndRegulatedContent,
        Label::DatingAndRomanticContent,
        Label::Profanity,
        Label::DisruptiveAudio,
    ];

    /// The name in the model's config.json (`ABUSE_TYPE_…`).
    pub const fn model_name(self) -> &'static str {
        match self {
            Label::PrivacyAskingForPii => "ABUSE_TYPE_PRIVACY_ASKING_FOR_PII",
            Label::Discriminatory => "ABUSE_TYPE_DISCRIMINATORY",
            Label::Harassment => "ABUSE_TYPE_HARASSMENT",
            Label::SexualContent => "ABUSE_TYPE_SEXUAL_CONTENT",
            Label::IllegalAndRegulatedContent => "ABUSE_TYPE_ILLEGAL_AND_REGULATED_CONTENT",
            Label::DatingAndRomanticContent => "ABUSE_TYPE_DATING_AND_ROMANTIC_CONTENT",
            Label::Profanity => "ABUSE_TYPE_PROFANITY",
            Label::DisruptiveAudio => "ABUSE_TYPE_DISRUPTIVE_AUDIO",
        }
    }

    /// Stable short key used in settings files and URLs.
    pub const fn key(self) -> &'static str {
        match self {
            Label::PrivacyAskingForPii => "privacy_asking_for_pii",
            Label::Discriminatory => "discriminatory",
            Label::Harassment => "harassment",
            Label::SexualContent => "sexual_content",
            Label::IllegalAndRegulatedContent => "illegal_and_regulated_content",
            Label::DatingAndRomanticContent => "dating_and_romantic_content",
            Label::Profanity => "profanity",
            Label::DisruptiveAudio => "disruptive_audio",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

impl FromStr for Label {
    type Err = UnknownLabel;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Label::ALL
            .into_iter()
            .find(|l| l.key() == s || l.model_name() == s)
            .ok_or_else(|| UnknownLabel(s.to_owned()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown label {0:?}")]
pub struct UnknownLabel(pub String);

/// The classifier's language heads (a spoken-language guess per sentence), in the model's output order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClfLang {
    Ar,
    Bg,
    Cs,
    Da,
    De,
    El,
    En,
    Es,
    Fi,
    Fr,
    Hr,
    Hu,
    Id,
    It,
    Ja,
    Ko,
    Nl,
    No,
    Pl,
    Pt,
    Ro,
    Ru,
    Sk,
    Sv,
    Th,
    Tl,
    Tr,
    Uk,
    Zh,
    /// Speech the model does not attribute to one of the 29 languages.
    Other,
}

impl ClfLang {
    pub const ALL: [ClfLang; 30] = [
        ClfLang::Ar,
        ClfLang::Bg,
        ClfLang::Cs,
        ClfLang::Da,
        ClfLang::De,
        ClfLang::El,
        ClfLang::En,
        ClfLang::Es,
        ClfLang::Fi,
        ClfLang::Fr,
        ClfLang::Hr,
        ClfLang::Hu,
        ClfLang::Id,
        ClfLang::It,
        ClfLang::Ja,
        ClfLang::Ko,
        ClfLang::Nl,
        ClfLang::No,
        ClfLang::Pl,
        ClfLang::Pt,
        ClfLang::Ro,
        ClfLang::Ru,
        ClfLang::Sk,
        ClfLang::Sv,
        ClfLang::Th,
        ClfLang::Tl,
        ClfLang::Tr,
        ClfLang::Uk,
        ClfLang::Zh,
        ClfLang::Other,
    ];

    /// The code in the model's config.json.
    pub const fn code(self) -> &'static str {
        match self {
            ClfLang::Ar => "ar",
            ClfLang::Bg => "bg",
            ClfLang::Cs => "cs",
            ClfLang::Da => "da",
            ClfLang::De => "de",
            ClfLang::El => "el",
            ClfLang::En => "en",
            ClfLang::Es => "es",
            ClfLang::Fi => "fi",
            ClfLang::Fr => "fr",
            ClfLang::Hr => "hr",
            ClfLang::Hu => "hu",
            ClfLang::Id => "id",
            ClfLang::It => "it",
            ClfLang::Ja => "ja",
            ClfLang::Ko => "ko",
            ClfLang::Nl => "nl",
            ClfLang::No => "no",
            ClfLang::Pl => "pl",
            ClfLang::Pt => "pt",
            ClfLang::Ro => "ro",
            ClfLang::Ru => "ru",
            ClfLang::Sk => "sk",
            ClfLang::Sv => "sv",
            ClfLang::Th => "th",
            ClfLang::Tl => "tl",
            ClfLang::Tr => "tr",
            ClfLang::Uk => "uk",
            ClfLang::Zh => "zh",
            ClfLang::Other => "other",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    pub fn from_code(code: &str) -> Option<ClfLang> {
        ClfLang::ALL.into_iter().find(|l| l.code() == code)
    }
}

impl fmt::Display for ClfLang {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_match_the_model_config() {
        let labels = [
            "ABUSE_TYPE_PRIVACY_ASKING_FOR_PII",
            "ABUSE_TYPE_DISCRIMINATORY",
            "ABUSE_TYPE_HARASSMENT",
            "ABUSE_TYPE_SEXUAL_CONTENT",
            "ABUSE_TYPE_ILLEGAL_AND_REGULATED_CONTENT",
            "ABUSE_TYPE_DATING_AND_ROMANTIC_CONTENT",
            "ABUSE_TYPE_PROFANITY",
            "ABUSE_TYPE_DISRUPTIVE_AUDIO",
        ];
        for (i, name) in labels.iter().enumerate() {
            assert_eq!(Label::ALL[i].model_name(), *name);
            assert_eq!(name.parse::<Label>().map(Label::index), Ok(i));
        }
        let langs = "ar bg cs da de el en es fi fr hr hu id it ja ko nl no pl pt ro ru sk sv th tl tr uk zh other";
        for (i, code) in langs.split(' ').enumerate() {
            assert_eq!(ClfLang::ALL[i].code(), code);
            assert_eq!(ClfLang::from_code(code).map(ClfLang::index), Some(i));
        }
    }
}
