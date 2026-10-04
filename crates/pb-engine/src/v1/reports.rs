//! Telling people: one mod-log post per flagged sentence (with a violation's step and action), the owner's direct
//! message when an escalation step says so, and the daily or weekly summary. All text in the community's (or the owner's) chat language. Nothing
//! is held back or capped; Fluxer's rate limits are waited out by the client.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use jiff::civil::{Time, Weekday};
use jiff::{SignedDuration, Timestamp, ToSpan, Zoned};
use pb_domain::GuildId;
use pb_fluxer_api::{Attachment, Destination, OutgoingMessage};
use pb_i18n::{Arg, Locale, duration, label_name, text};
use pb_settings::{Digest, EscalationStep};
use pb_store_api::{ActionRecord, DecisionRecord, Event, MessagePurpose, MessageSent, SentenceRecord};

use super::core::Core;

/// The chat language of a community (or the global one).
pub fn locale(core: &Core, guild: Option<GuildId>) -> Locale {
    Locale::for_lang(&core.settings.current().effective(guild, None).chat_language.value)
}

/// Sends a message and records it; `false` when it could not be sent.
pub async fn send(
    core: &Core,
    to: Destination,
    content: String,
    file: Option<(String, Bytes)>,
    purpose: MessagePurpose,
    guild: Option<GuildId>,
) -> bool {
    let Some(ctl) = core.ctl() else { return false };
    let with_audio = file.is_some();
    let files = file
        .map(|(name, bytes)| {
            vec![Attachment {
                filename: name,
                media_type: "audio/wav".into(),
                bytes,
            }]
        })
        .unwrap_or_default();
    let m = OutgoingMessage {
        content,
        reply_to: None,
        ping: vec![],
        files,
    };
    let channel = match to {
        Destination::Channel(c) => Some(c),
        Destination::User(_) => None,
    };
    let r = ctl.send(to, m).await;
    let (ok, error) = match &r {
        Ok(_) => (true, None),
        Err(e) => {
            tracing::warn!(error = %e, "a message could not be sent");
            (false, Some(e.to_string()))
        }
    };
    core.record(vec![Event::MessageSent(MessageSent {
        purpose,
        guild,
        channel,
        ok,
        error,
        with_audio,
    })])
    .await;
    ok
}

fn decision_key(d: &DecisionRecord) -> &'static str {
    match d {
        DecisionRecord::Warn { .. } => "warn",
        DecisionRecord::Observe { .. } => "observe",
        DecisionRecord::Late { .. } => "late",
        DecisionRecord::Strike { .. } => "strike",
        _ => "other",
    }
}

fn file_name(prefix: &str, s: &SentenceRecord) -> String {
    format!("{prefix}-{}-{}.wav", s.user, s.started.strftime("%Y%m%d-%H%M%S"))
}

/// Tells about a flagged sentence of a tracked person, once its decision (and a violation's action) is known: one post
/// in the community's mod log, when it has one, and for a violation whose step says so a direct message to the owner.
pub async fn flagged(
    core: &Arc<Core>,
    s: &SentenceRecord,
    step: Option<&EscalationStep>,
    action: Option<&ActionRecord>,
    wav: &Bytes,
) {
    modlog(core, s, action, wav).await;
    if step.is_some_and(|st| st.notify_owner) {
        owner_dm(core, s, action, wav).await;
    }
}

/// A violation's count, window and step, for messages.
fn violation_args(core: &Core, s: &SentenceRecord, loc: Locale) -> Option<Vec<(&'static str, Arg)>> {
    let (label, score, step, count) = s.decision.violation()?;
    let eff = core.settings.current().effective(Some(s.guild), Some(s.user));
    Some(vec![
        ("label", label_name(loc, label).into()),
        ("score", loc.fixed(f64::from(score), 2).into()),
        ("count", count.into()),
        (
            "window",
            duration(loc, eff.violation_window.value.value().map(|d| d.get().get())).into(),
        ),
        ("step", step.into()),
        ("decision", decision_key(&s.decision).into()),
    ])
}

async fn modlog(core: &Arc<Core>, s: &SentenceRecord, action: Option<&ActionRecord>, wav: &Bytes) {
    let eff = core.settings.current().effective(Some(s.guild), None);
    let Some(channel) = eff.modlog_channel.value else {
        return;
    };
    let loc = locale(core, Some(s.guild));
    let labels: Vec<String> = s
        .flagged
        .iter()
        .map(|l| {
            let bar = s.thresholds.iter().find(|(x, _)| x == l).map_or(0.0, |(_, t)| *t);
            text(
                loc,
                "modlog-label-score",
                &[
                    ("label", label_name(loc, *l).into()),
                    ("score", loc.fixed(f64::from(s.scores[l.index()]), 2).into()),
                    ("bar", loc.fixed(f64::from(bar), 2).into()),
                ],
            )
        })
        .collect();
    let (strike, of) = match s.decision {
        DecisionRecord::Strike { strike, of } => (strike, of),
        _ => (0, 0),
    };
    let mut content = text(
        loc,
        "modlog-flagged",
        &[
            ("user", format!("<@{}>", s.user).into()),
            ("channel", format!("<#{}>", s.channel).into()),
            ("labels", labels.join(" · ").into()),
            ("seconds", loc.decimal(f64::from(s.dur_ms) / 1000.0, 1).into()),
            ("language", s.language.code().into()),
            ("decision", decision_key(&s.decision).into()),
            ("strike", strike.into()),
            ("of", of.into()),
        ],
    );
    if let Some(args) = violation_args(core, s, loc) {
        content.push_str(&text(loc, "modlog-violation", &args));
    }
    if let Some(a) = action {
        content.push_str(&action_text(loc, a));
    }
    let file = eff.modlog_audio.value.then(|| (file_name("flagged", s), wav.clone()));
    send(
        core,
        Destination::Channel(channel),
        content,
        file,
        MessagePurpose::Modlog { sentence: s.id },
        Some(s.guild),
    )
    .await;
}

async fn owner_dm(core: &Arc<Core>, s: &SentenceRecord, action: Option<&ActionRecord>, wav: &Bytes) {
    let Some(owner) = core.owner() else { return };
    let loc = locale(core, None);
    let Some(mut args) = violation_args(core, s, loc) else {
        return;
    };
    let (name, community, channel) = {
        let gs = core.guilds();
        (
            gs.name(s.guild, s.user),
            gs.guild_name(s.guild),
            gs.channel_name(s.guild, s.channel),
        )
    };
    args.extend([
        ("user", name.into()),
        ("community", community.into()),
        ("channel", format!("#{channel}").into()),
    ]);
    let mut content = text(loc, "violation", &args);
    if let Some(a) = action {
        content.push_str(&action_text(loc, a));
    }
    let audio = core
        .settings
        .current()
        .effective(Some(s.guild), Some(s.user))
        .owner_dm_audio
        .value;
    send(
        core,
        Destination::User(owner),
        content,
        audio.then(|| (file_name("violation", s), wav.clone())),
        MessagePurpose::OwnerDm { sentence: s.id },
        Some(s.guild),
    )
    .await;
}

/// The text of an action's result.
pub fn action_text(loc: Locale, a: &ActionRecord) -> String {
    let action = pb_i18n::action_text(loc, a.kind, a.secs);
    let result = pb_i18n::action_outcome(loc, &a.outcome);
    text(
        loc,
        "violation-action",
        &[("action", action.into()), ("result", result.into())],
    )
}

/// The summary report's schedule: the latest scheduled moment at or before `now`.
pub fn last_slot(now: &Zoned, mode: Digest, at: Time, weekday: Weekday) -> Option<Zoned> {
    if mode == Digest::Off {
        return None;
    }
    let today = now
        .date()
        .to_zoned(now.time_zone().clone())
        .ok()?
        .with()
        .time(at)
        .build()
        .ok()?;
    let mut slot = today;
    if mode == Digest::Weekly {
        let back = (slot.weekday().to_monday_zero_offset() - weekday.to_monday_zero_offset()).rem_euclid(7);
        slot = slot.checked_sub(i64::from(back).days()).ok()?;
    }
    if slot > *now {
        slot = slot
            .checked_sub(if mode == Digest::Weekly { 7.days() } else { 1.days() })
            .ok()?;
    }
    Some(slot)
}

fn weekday(w: pb_settings::Weekday) -> Weekday {
    match w {
        pb_settings::Weekday::Monday => Weekday::Monday,
        pb_settings::Weekday::Tuesday => Weekday::Tuesday,
        pb_settings::Weekday::Wednesday => Weekday::Wednesday,
        pb_settings::Weekday::Thursday => Weekday::Thursday,
        pb_settings::Weekday::Friday => Weekday::Friday,
        pb_settings::Weekday::Saturday => Weekday::Saturday,
        pb_settings::Weekday::Sunday => Weekday::Sunday,
    }
}

/// Sends the summary when one is due; checks every minute.
pub async fn digest_scheduler(core: Arc<Core>) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        if let Err(e) = digest_once(&core, false).await {
            tracing::warn!(error = %e, "the summary report failed");
        }
    }
}

/// Sends the summary if due (or now, `force`, covering the time since the last one). Returns whether one was sent.
pub async fn digest_once(core: &Arc<Core>, force: bool) -> Result<bool, super::error::EngineError> {
    let eff = core.settings.current().effective(None, None);
    let tz = eff.timezone.value.zone();
    let now = core.deps.clock.now().to_zoned(tz);
    // The setting only holds valid times of day.
    let at = Time::new(
        i8::try_from(eff.digest_time.value.hour).unwrap_or(9),
        i8::try_from(eff.digest_time.value.minute).unwrap_or(0),
        0,
        0,
    )
    .unwrap_or(Time::constant(9, 0, 0, 0));
    // A report that could not be delivered still settles its slot (no retry every minute); the next one covers its
    // time as well.
    let last = core.deps.index.last_digest().await?;
    let (from, until) = if force {
        let from = last.sent.unwrap_or_else(|| {
            now.timestamp()
                .checked_sub(SignedDuration::from_hours(24))
                .unwrap_or(now.timestamp())
        });
        (from, now.timestamp())
    } else {
        let Some(slot) = last_slot(&now, eff.digest.value, at, weekday(eff.digest_weekday.value)) else {
            return Ok(false);
        };
        if last.tried.is_some_and(|l| l >= slot.timestamp()) {
            return Ok(false);
        }
        let period = if eff.digest.value == Digest::Weekly {
            SignedDuration::from_hours(24 * 7)
        } else {
            SignedDuration::from_hours(24)
        };
        (
            last.sent
                .unwrap_or_else(|| slot.timestamp().checked_sub(period).unwrap_or(slot.timestamp())),
            slot.timestamp(),
        )
    };
    let Some(owner) = core.owner() else { return Ok(false) };
    let rows = core.deps.index.digest(from, until).await?;
    let loc = locale(core, None);
    let day = |t: Timestamp| t.to_zoned(eff.timezone.value.zone()).date().to_string();
    let mut lines = vec![text(
        loc,
        "digest-head",
        &[("from", day(from).into()), ("until", day(until).into())],
    )];
    if rows.is_empty() {
        lines.push(text(loc, "digest-none", &[]));
    } else {
        let people = rows
            .iter()
            .map(|r| (r.guild, r.user))
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let total: u32 = rows.iter().map(|r| r.violations).sum();
        lines.push(text(
            loc,
            "digest-summary",
            &[("violations", total.into()), ("people", people.into())],
        ));
        for r in &rows {
            let (name, community) = {
                let gs = core.guilds();
                (gs.name(r.guild, r.user), gs.guild_name(r.guild))
            };
            let jar = core.jar(r.guild, r.user);
            lines.push(text(
                loc,
                "digest-person",
                &[
                    ("user", name.into()),
                    ("community", community.into()),
                    ("count", r.violations.into()),
                    ("label", label_name(loc, r.top_label).into()),
                    ("step", r.max_step.into()),
                    ("actions", Arg::Number(f64::from(r.actions))),
                    (
                        "jar",
                        if jar == 0 {
                            "none".into()
                        } else {
                            jar.to_string().into()
                        },
                    ),
                ],
            ));
        }
    }
    let ok = send(
        core,
        Destination::User(owner),
        lines.join("\n"),
        None,
        MessagePurpose::Digest { from, until },
        None,
    )
    .await;
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Zoned {
        s.parse().unwrap()
    }

    #[test]
    fn the_last_report_slot() {
        let nine = Time::new(9, 0, 0, 0).unwrap();
        // Daily: today's slot once it has passed, else yesterday's.
        let s = last_slot(
            &at("2026-10-07T10:00[Europe/Berlin]"),
            Digest::Daily,
            nine,
            Weekday::Monday,
        )
        .unwrap();
        assert_eq!(s.to_string(), "2026-10-07T09:00:00+02:00[Europe/Berlin]");
        let s = last_slot(
            &at("2026-10-07T08:00[Europe/Berlin]"),
            Digest::Daily,
            nine,
            Weekday::Monday,
        )
        .unwrap();
        assert_eq!(s.date().to_string(), "2026-10-06");
        // Weekly on Mondays: 2026-10-07 is a Wednesday, so the last Monday is 10-05.
        let s = last_slot(
            &at("2026-10-07T10:00[Europe/Berlin]"),
            Digest::Weekly,
            nine,
            Weekday::Monday,
        )
        .unwrap();
        assert_eq!(s.date().to_string(), "2026-10-05");
        let s = last_slot(
            &at("2026-10-05T08:59[Europe/Berlin]"),
            Digest::Weekly,
            nine,
            Weekday::Monday,
        )
        .unwrap();
        assert_eq!(
            s.date().to_string(),
            "2026-09-28",
            "before the time on the day itself: last week"
        );
        assert!(last_slot(&at("2026-10-07T10:00[UTC]"), Digest::Off, nine, Weekday::Monday).is_none());
    }
}
