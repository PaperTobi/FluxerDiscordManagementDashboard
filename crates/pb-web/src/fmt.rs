//! Formatting shared by pages and islands.

use pb_i18n::{Locale, label_name, text};
use pb_live_proto::{DecisionView, Station};

/// A score as a percentage (`87 %`).
pub fn pct(score: f32) -> String {
    let p = (score * 100.0).round() as i32;
    format!("{p} %")
}

/// Milliseconds as people read them (`850 ms`, `1.2 s`).
pub fn ms(loc: Locale, v: u64) -> String {
    if v < 1000 {
        format!("{v} ms")
    } else {
        format!("{} s", loc.decimal(v as f64 / 1000.0, 1))
    }
}

/// How long ago, from two millisecond timestamps (`now`, `then`).
pub fn ago(loc: Locale, now: i64, then: i64) -> String {
    let s = ((now - then).max(0) / 1000) as u64;
    let n = |id: &str, v: u64| text(loc, id, &[("n", v.into())]);
    match s {
        0..=4 => text(loc, "ui-ago-now", &[]),
        5..=59 => n("ui-ago-s", s),
        60..=3599 => n("ui-ago-min", s / 60),
        3600..=86_399 => n("ui-ago-h", s / 3600),
        _ => n("ui-ago-d", s / 86_400),
    }
}

/// Why a sentence counted: the first type it was flagged for, with its score (`None`: nothing was flagged; the highest
/// score of a clean sentence says nothing).
pub fn flagged(v: &pb_live_proto::VerdictView) -> Option<(pb_domain::Label, f32)> {
    v.flagged.first().map(|l| (*l, v.score(*l)))
}

/// A station's name.
pub fn station(loc: Locale, s: Station) -> String {
    let id = match s {
        Station::Recording => "ui-station-recording",
        Station::Cut => "ui-station-cut",
        Station::Queued => "ui-station-queued",
        Station::Model => "ui-station-model",
        Station::Verdict => "ui-station-verdict",
        Station::Decision => "ui-station-decision",
    };
    text(loc, id, &[])
}

/// What was decided, briefly.
pub fn decision(loc: Locale, d: DecisionView) -> String {
    match d {
        DecisionView::NothingFlagged => text(loc, "ui-decision-clear", &[]),
        DecisionView::InvalidScore => text(loc, "ui-decision-invalid", &[]),
        DecisionView::NoLongerTracked => text(loc, "ui-decision-untracked", &[]),
        DecisionView::Strike { strike, of } => text(
            loc,
            "ui-decision-strike",
            &[("strike", strike.into()), ("of", of.into())],
        ),
        DecisionView::Warn { step, .. } => text(loc, "ui-decision-warn", &[("step", step.into())]),
        DecisionView::Observe { step, .. } => text(loc, "ui-decision-observe", &[("step", step.into())]),
        DecisionView::Late { step, .. } => text(loc, "ui-decision-late", &[("step", step.into())]),
    }
}

/// The CSS class of a decision.
pub fn decision_class(d: DecisionView) -> &'static str {
    match d {
        DecisionView::NothingFlagged => "ok",
        DecisionView::InvalidScore | DecisionView::NoLongerTracked => "muted",
        DecisionView::Strike { .. } => "warn",
        DecisionView::Warn { .. } | DecisionView::Late { .. } => "bad",
        DecisionView::Observe { .. } => "warn",
    }
}

/// A detection type's short name.
pub fn label(loc: Locale, l: pb_domain::Label) -> String {
    label_name(loc, l)
}

/// The initial of a name (for avatars without a picture).
pub fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_else(|| "?".into())
}

/// A language by its own name (the same in every UI language), else its code.
pub fn language(code: &str) -> String {
    let name = match code {
        "ar" => "العربية",
        "bg" => "Български",
        "cs" => "Čeština",
        "da" => "Dansk",
        "de" => "Deutsch",
        "el" => "Ελληνικά",
        "en" => "English",
        "es" => "Español",
        "fi" => "Suomi",
        "fr" => "Français",
        "hi" => "हिन्दी",
        "hu" => "Magyar",
        "id" => "Bahasa Indonesia",
        "it" => "Italiano",
        "ja" => "日本語",
        "ko" => "한국어",
        "nl" => "Nederlands",
        "no" | "nb" => "Norsk",
        "pl" => "Polski",
        "pt" => "Português",
        "ro" => "Română",
        "ru" => "Русский",
        "sv" => "Svenska",
        "th" => "ไทย",
        "tl" => "Tagalog",
        "tr" => "Türkçe",
        "uk" => "Українська",
        "vi" => "Tiếng Việt",
        "zh" => "中文",
        "ms" => "Bahasa Melayu",
        "he" => "עברית",
        "ca" => "Català",
        "fa" => "فارسی",
        "sk" => "Slovenčina",
        "sl" => "Slovenščina",
        "sr" => "Српски",
        "hr" => "Hrvatski",
        "lt" => "Lietuvių",
        "lv" => "Latviešu",
        "et" => "Eesti",
        "is" => "Íslenska",
        "cy" => "Cymraeg",
        "ga" => "Gaeilge",
        "sw" => "Kiswahili",
        "kk" => "Қазақ",
        "ka" => "ქართული",
        "ne" => "नेपाली",
        "lb" => "Lëtzebuergesch",
        _ => return code.to_owned(),
    };
    format!("{name} ({code})")
}

/// An escalation window (`None` = unlimited).
pub fn window(loc: Locale, ms: Option<u64>) -> String {
    pb_i18n::duration(loc, ms.map(std::time::Duration::from_millis))
}

/// Why a sentence was not scored.
pub fn dropped(loc: Locale, d: pb_live_proto::DropWhy) -> String {
    let id = match d {
        pb_live_proto::DropWhy::TooLittleSpeech => "ui-dropped-short",
        pb_live_proto::DropWhy::OwnPlayback => "ui-dropped-echo",
    };
    pb_i18n::text(loc, id, &[])
}

/// A size in bytes, in binary units.
pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < UNITS.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// The names (as Fluxer shows them) of the permissions in `need` that `have` lacks.
/// A local path to go back to (never another site). Browsers drop tabs and line breaks from `Location` and read `\`
/// as `/`, so a path with any of those (or other control characters or spaces) is refused as a whole: `/` instead.
pub fn local_path(next: Option<&str>) -> String {
    match next {
        Some(n)
            if n.starts_with('/')
                && !n.starts_with("//")
                && !n.chars().any(|c| c == '\\' || c.is_whitespace() || c.is_control()) =>
        {
            n.to_owned()
        }
        _ => "/".to_owned(),
    }
}

#[cfg(feature = "ssr")]
pub fn missing_permissions(loc: Locale, have: u64, need: u64) -> Vec<String> {
    pb_fluxer_api::perms::missing(have, need)
        .into_iter()
        .map(|n| pb_i18n::permission_name(loc, n))
        .collect()
}

#[cfg(feature = "ssr")]
/// Why the engine did not do what was asked, in the page's language.
pub fn engine_error(loc: Locale, e: &pb_engine::EngineError) -> String {
    use pb_engine::EngineError as E;
    use pb_i18n::text;
    use pb_store_api::PlayOutcome as P;
    let with = |id: &str, error: &str| text(loc, id, &[("error", error.to_owned().into())]);
    match e {
        E::NotConnected => text(loc, "err-not-connected", &[]),
        E::NotInCall => text(loc, "err-not-in-call", &[]),
        E::BotNotInCall => text(loc, "err-bot-not-in-call", &[]),
        E::NoLanguage => text(loc, "err-no-language", &[]),
        E::NotSaid(P::TooLate) => text(loc, "err-said-too-late", &[]),
        E::NotSaid(P::NotSpoken) => text(loc, "err-said-not-spoken", &[]),
        E::NotSaid(P::Failed { error }) => with("err-said-failed", error),
        E::NotSaid(_) => text(loc, "err-said-nothing", &[]),
        E::NoSuchClip => text(loc, "ui-no-such-clip", &[]),
        E::NoSuchVoice => text(loc, "err-no-such-voice", &[]),
        E::Voice(error) => voice_error(loc, error),
        E::Unreadable(error) => with("ui-clip-unreadable", &error.to_string()),
        E::NoSuchSentence => text(loc, "err-no-such-sentence", &[]),
        E::NoRecording => text(loc, "err-no-recording", &[]),
        E::LogHalted => text(loc, "err-log-halted", &[]),
        E::Render(error) => render_error(loc, error),
        E::Fluxer(error) => with("err-fluxer", &error.to_string()),
        E::Login(error) => {
            use pb_fluxer_api::LoginError as L;
            match error {
                L::TokenRejected => text(loc, "setup-token-rejected", &[]),
                L::BadInstance(why) => with("err-bad-instance", why),
                L::Unreachable(why) => with("login-unreachable", why),
                L::Refused(why) => with("err-fluxer", why),
            }
        }
        E::Store(error) => store_error(loc, error),
    }
}

#[cfg(feature = "ssr")]
/// Why a voice could not be made from a sample, in the page's language.
fn voice_error(loc: Locale, e: &pb_engine::VoiceError) -> String {
    use pb_engine::VoiceError as V;
    use pb_i18n::text;
    match e {
        V::NoCloning => text(loc, "err-voice-no-cloning", &[]),
        V::NeedsTranscript => text(loc, "err-voice-needs-transcript", &[]),
        V::NoModel(model) => text(loc, "err-voice-no-model", &[("model", model.clone().into())]),
        V::Failed(error) => text(loc, "err-voice", &[("error", error.clone().into())]),
    }
}

#[cfg(feature = "ssr")]
/// Why speech or a clip could not be made, in the page's language.
pub fn render_error(loc: Locale, e: &pb_engine::RenderError) -> String {
    use pb_engine::RenderError as R;
    use pb_i18n::text;
    match e {
        R::NoVoice(lang) => text(loc, "err-render-no-voice", &[("lang", lang.to_string().into())]),
        R::ClipMissing(_) => text(loc, "err-render-clip-missing", &[]),
        R::ClipUnreadable { error, .. } => text(loc, "ui-clip-unreadable", &[("error", error.to_string().into())]),
        R::Store(error) => store_error(loc, error),
        R::Tts(error) => text(loc, "err-render", &[("error", error.to_string().into())]),
    }
}

#[cfg(feature = "ssr")]
/// A storage error, in the page's language (the cause in the system's words).
pub fn store_error(loc: Locale, e: &dyn std::fmt::Display) -> String {
    pb_i18n::text(loc, "err-store", &[("error", e.to_string().into())])
}
