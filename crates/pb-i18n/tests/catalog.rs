//! The built-in catalog is complete: every locale has every message, and every setting, section, detection type and
//! choice has a name.

#![allow(clippy::unwrap_used, clippy::expect_used)] // a panic is how a test fails

use pb_domain::Label;
use pb_i18n::{Arg, Catalog, Locale, choice_id, duration, label_id, section_id, setting_id, text};
use pb_settings::{AudienceChoice, Digest, NoSpeakPolicy, Recordings, Section, SettingKey, StepAction, Weekday};
use std::time::Duration;

#[test]
fn loads_without_problems() {
    assert_eq!(Catalog::get().problems(), &[]);
}

#[test]
fn every_locale_has_the_same_messages() {
    let c = Catalog::get();
    let en = c.ids(Locale::En);
    assert!(!en.is_empty());
    for locale in Locale::ALL {
        let mut a: Vec<_> = en.to_vec();
        let mut b: Vec<_> = c.ids(locale).to_vec();
        a.sort_by(|x, y| x.id.cmp(&y.id));
        b.sort_by(|x, y| x.id.cmp(&y.id));
        assert_eq!(a, b, "{} differs from en", locale.tag());
    }
}

fn need(id: &str, attr: Option<&str>) {
    for locale in Locale::ALL {
        let c = Catalog::get();
        assert!(c.has(locale, id), "{}: missing {id}", locale.tag());
        if let Some(a) = attr {
            assert!(
                c.ids(locale)
                    .iter()
                    .any(|m| m.id == id && m.attributes.iter().any(|x| x == a)),
                "{}: {id} has no .{a}",
                locale.tag()
            );
        }
    }
}

#[test]
fn every_setting_section_label_and_choice_is_named() {
    for key in SettingKey::all() {
        need(&setting_id(key), Some("help"));
        for locale in Locale::ALL {
            let name = pb_i18n::setting_name(locale, key);
            assert!(!name.starts_with("setting-"), "{name}");
            assert!(!pb_i18n::setting_help(locale, key).is_empty());
        }
    }
    for s in Section::ALL {
        need(&section_id(s), None);
    }
    for l in Label::ALL {
        need(&label_id(l), None);
    }
    let choices: [(&str, Vec<&str>); 6] = [
        ("audience", AudienceChoice::ALL.iter().map(|c| c.as_str()).collect()),
        (
            "no_speak_policy",
            NoSpeakPolicy::ALL.iter().map(|c| c.as_str()).collect(),
        ),
        ("digest", Digest::ALL.iter().map(|c| c.as_str()).collect()),
        ("weekday", Weekday::ALL.iter().map(|c| c.as_str()).collect()),
        ("recordings", Recordings::ALL.iter().map(|c| c.as_str()).collect()),
        ("step_action", StepAction::ALL.iter().map(|c| c.as_str()).collect()),
    ];
    for (kind, values) in choices {
        for v in values {
            need(&choice_id(kind, v), None);
        }
    }
}

#[test]
fn formats_with_plurals_and_no_isolation_marks() {
    let c = Catalog::get();
    let args = |n: u32| vec![("count", Arg::from(n)), ("people", Arg::from("<@1>"))];
    assert_eq!(
        c.try_format(Locale::En, "cmd-status-following", None, &args(1))
            .as_deref(),
        Ok("Following one person: <@1>")
    );
    assert_eq!(
        c.try_format(Locale::De, "cmd-status-following", None, &args(3))
            .as_deref(),
        Ok("Folgt 3 Personen: <@1>")
    );
    assert_eq!(
        c.try_format(
            Locale::En,
            "cmd-status-following",
            None,
            &[("count", Arg::from(2u32)), ("people", Arg::from("none"))]
        )
        .as_deref(),
        Ok("Following 2 people")
    );
    assert_eq!(
        text(
            Locale::En,
            "cmd-set-person",
            &[
                ("setting", "Strikes".into()),
                ("value", "3".into()),
                ("user", "<@5>".into())
            ]
        ),
        "Strikes set to 3 for <@5>."
    );
    assert!(
        c.try_format(Locale::En, "cmd-set-person", None, &[]).is_err(),
        "missing arguments are reported"
    );
    assert!(c.try_format(Locale::En, "no-such-message", None, &[]).is_err());
    assert_eq!(text(Locale::De, "no-such-message", &[]), "no-such-message");
}

#[test]
fn help_variants() {
    for locale in Locale::ALL {
        for audio in [true, false] {
            let out = Catalog::get()
                .try_format(
                    locale,
                    "cmd-help",
                    None,
                    &[("prefix", "!pb".into()), ("audio", audio.into())],
                )
                .unwrap_or_else(|e| panic!("{e:?}"));
            assert!(out.contains("`!pb add @user…`"));
        }
    }
}

#[test]
fn durations() {
    let s = Duration::from_secs;
    assert_eq!(duration(Locale::En, None), "unlimited");
    assert_eq!(duration(Locale::De, None), "unbegrenzt");
    assert_eq!(duration(Locale::En, Some(Duration::from_millis(600))), "600 ms");
    assert_eq!(duration(Locale::En, Some(Duration::from_millis(1500))), "1.5 s");
    assert_eq!(duration(Locale::De, Some(Duration::from_millis(1500))), "1,5 s");
    assert_eq!(duration(Locale::En, Some(s(20 * 60))), "20 min");
    assert_eq!(duration(Locale::En, Some(s(3600))), "1 h");
    assert_eq!(duration(Locale::En, Some(s(86_400))), "1 day");
    assert_eq!(duration(Locale::De, Some(s(3 * 86_400))), "3 Tage");
}

#[test]
fn locales() {
    assert_eq!(Locale::negotiate("de-DE,de;q=0.9,en;q=0.8"), Locale::De);
    assert_eq!(Locale::negotiate("fr-FR, en;q=0.5, de;q=0.7"), Locale::De);
    assert_eq!(Locale::negotiate("fr"), Locale::En);
    assert_eq!(Locale::negotiate(""), Locale::En);
    assert_eq!(
        Locale::for_lang(&"de-AT".parse().unwrap_or_else(|_| unreachable!())),
        Locale::De
    );
    assert_eq!(Locale::De.fixed(0.6, 2), "0,60");
    assert_eq!(Locale::En.decimal(2.0, 2), "2");
}
