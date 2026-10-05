//! Folding events into the tables.

use pb_domain::{ActionOutcome, BlobHash, Scope};
use pb_settings::Change;
use pb_store_api::{AUDIT_KINDS, Event, MessagePurpose, StoreError, StoredEvent};

use super::db::{Db, opt};

fn ms(ts: jiff::Timestamp) -> i64 {
    ts.as_millisecond()
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_default()
}

/// The community and person an audited event is about.
fn audit_scope(e: &Event) -> (Option<String>, Option<String>) {
    let scope = |s: &Scope| match *s {
        Scope::Global => (None, None),
        Scope::Server { guild } => (Some(guild.to_string()), None),
        Scope::Person { guild, user } => (Some(guild.to_string()), Some(user.to_string())),
    };
    match e {
        Event::SettingsChanged(c) => match &c.change {
            Change::Set { scope: s, .. } | Change::Clear { scope: s, .. } | Change::VoiceLine { scope: s, .. } => {
                scope(s)
            }
            Change::Track { guild, user, .. } | Change::Untrack { guild, user } => {
                (Some(guild.to_string()), Some(user.to_string()))
            }
        },
        Event::Action(a) => (Some(a.guild.to_string()), Some(a.user.to_string())),
        Event::JarReset(j) => (Some(j.guild.to_string()), Some(j.user.to_string())),
        Event::JarBaseline(j) => (Some(j.guild.to_string()), Some(j.user.to_string())),
        Event::MessageSent(m) => (m.guild.map(|g| g.to_string()), None),
        Event::Login(l) => (None, Some(l.user.to_string())),
        Event::ImportedAudit(a) => (a.guild.map(|g| g.to_string()), a.user.map(|u| u.to_string())),
        _ => (None, None),
    }
}

async fn add_jar(db: &Db, guild: String, user: String, n: i64) -> Result<(), StoreError> {
    db.exec(
        "INSERT INTO jar (guild, user, count) VALUES (?1, ?2, ?3) \
         ON CONFLICT (guild, user) DO UPDATE SET count = count + excluded.count",
        vec![guild.into(), user.into(), n.into()],
    )
    .await
    .map(|_| ())
}

/// Applies one event (inside the caller's transaction).
pub async fn apply(db: &Db, stored: &StoredEvent) -> Result<(), StoreError> {
    let event = match Event::from_stored(stored) {
        Ok(e) => e,
        Err(e) => {
            // The log is the truth; an event the index cannot read is left out (and counted on the System page).
            tracing::warn!(error = %e, "the index skips an unreadable event");
            let n = db
                .meta("skipped")
                .await?
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            return db.set_meta("skipped", &(n + 1).to_string()).await;
        }
    };
    let seq = i64::try_from(stored.seq).unwrap_or(i64::MAX);
    let ts = ms(stored.ts);
    if AUDIT_KINDS.contains(&stored.kind.as_str()) {
        let (guild, user) = audit_scope(&event);
        db.exec(
            "INSERT INTO audit (seq, ts_ms, kind, v, guild, user, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            vec![
                seq.into(),
                ts.into(),
                stored.kind.clone().into(),
                i64::from(stored.v).into(),
                opt(guild),
                opt(user),
                stored.data.to_string().into(),
            ],
        )
        .await?;
    }
    match event {
        Event::Sentence(r) => {
            let violation = r.decision.violation();
            db.exec(
                "INSERT INTO sentences (seq, id, ts_ms, guild, user, started_ms, dur_ms, flagged, violation, label, step, \
                 audio, data) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                vec![
                    seq.into(),
                    r.id.to_string().into(),
                    ts.into(),
                    r.guild.to_string().into(),
                    r.user.to_string().into(),
                    ms(r.started).into(),
                    i64::from(r.dur_ms).into(),
                    i64::from(!r.flagged.is_empty()).into(),
                    i64::from(violation.is_some()).into(),
                    opt(violation.map(|v| v.0.key().to_owned())),
                    opt(violation.map(|v| i64::from(v.2))),
                    opt(r.audio.as_ref().map(BlobHash::hex)),
                    json(&*r).into(),
                ],
            )
            .await?;
            if r.jar && violation.is_some() {
                add_jar(db, r.guild.to_string(), r.user.to_string(), 1).await?;
            }
        }
        Event::Action(a) => {
            let done = matches!(a.outcome, ActionOutcome::Done);
            db.exec(
                "INSERT INTO actions (seq, id, ts_ms, guild, user, done, due_ms, undoes, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                vec![
                    seq.into(),
                    a.id.to_string().into(),
                    ts.into(),
                    a.guild.to_string().into(),
                    a.user.to_string().into(),
                    i64::from(done).into(),
                    opt(if done { a.undo_at.map(ms) } else { None }),
                    opt(a.undoes.map(|u| u.to_string())),
                    json(&*a).into(),
                ],
            )
            .await?;
            if let Some(undoes) = a.undoes {
                if done {
                    db.exec(
                        "UPDATE actions SET undone = 1 WHERE id = ?1",
                        vec![undoes.to_string().into()],
                    )
                    .await?;
                } else if let Some(retry) = a.retry_at {
                    db.exec(
                        "UPDATE actions SET due_ms = ?2 WHERE id = ?1",
                        vec![undoes.to_string().into(), ms(retry).into()],
                    )
                    .await?;
                } else {
                    // Given up: no longer pending.
                    db.exec(
                        "UPDATE actions SET undone = 2 WHERE id = ?1",
                        vec![undoes.to_string().into()],
                    )
                    .await?;
                }
            }
        }
        Event::JarReset(j) => {
            db.exec(
                "INSERT INTO jar (guild, user, count) VALUES (?1, ?2, 0) ON CONFLICT (guild, user) DO UPDATE SET count = 0",
                vec![j.guild.to_string().into(), j.user.to_string().into()],
            )
            .await?;
        }
        Event::JarBaseline(j) => {
            add_jar(
                db,
                j.guild.to_string(),
                j.user.to_string(),
                i64::try_from(j.count).unwrap_or(i64::MAX),
            )
            .await?;
        }
        Event::BlobDeleted(b) => {
            db.exec(
                "UPDATE sentences SET audio_deleted = 1 WHERE audio = ?1",
                vec![b.hash.hex().into()],
            )
            .await?;
        }
        Event::ClipSaved(c) => {
            db.exec(
                "INSERT INTO clips (render, seq, added_ms, removed, data) VALUES (?1, ?2, ?3, 0, ?4) \
                 ON CONFLICT (render) DO UPDATE SET seq = excluded.seq, removed = 0, data = excluded.data",
                vec![c.render.hex().into(), seq.into(), ts.into(), json(&*c).into()],
            )
            .await?;
        }
        Event::ClipRemoved(c) => {
            db.exec(
                "UPDATE clips SET removed = 1 WHERE render = ?1",
                vec![c.render.hex().into()],
            )
            .await?;
        }
        Event::VoiceSaved(v) => {
            // A rename keeps the time the voice was added.
            db.exec(
                "INSERT INTO voices (model, id, seq, added_ms, removed, data) VALUES (?1, ?2, ?3, ?4, 0, ?5) \
                 ON CONFLICT (model, id) DO UPDATE SET seq = excluded.seq, removed = 0, data = excluded.data",
                vec![
                    v.model.clone().into(),
                    v.id.clone().into(),
                    seq.into(),
                    ts.into(),
                    json(&*v).into(),
                ],
            )
            .await?;
        }
        Event::VoiceRemoved(v) => {
            db.exec(
                "UPDATE voices SET removed = 1 WHERE model = ?1 AND id = ?2",
                vec![v.model.clone().into(), v.id.clone().into()],
            )
            .await?;
        }
        Event::PersonSeen(p) => {
            db.exec(
                "INSERT INTO people (user, username, display_name, avatar) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (user) DO UPDATE SET username = excluded.username, display_name = excluded.display_name, \
                 avatar = excluded.avatar",
                vec![
                    p.user.to_string().into(),
                    p.username.clone().into(),
                    opt(p.display_name.clone()),
                    opt(p.avatar.clone()),
                ],
            )
            .await?;
            if let Some(g) = p.guild {
                db.exec(
                    "INSERT INTO nicks (guild, user, nick) VALUES (?1, ?2, ?3) ON CONFLICT (guild, user) DO UPDATE SET nick = excluded.nick",
                    vec![g.to_string().into(), p.user.to_string().into(), opt(p.nick.clone())],
                )
                .await?;
            }
        }
        Event::CommunitySeen(c) => {
            db.exec(
                "INSERT INTO communities (guild, name, icon) VALUES (?1, ?2, ?3) \
                 ON CONFLICT (guild) DO UPDATE SET name = excluded.name, icon = excluded.icon",
                vec![c.guild.to_string().into(), c.name.clone().into(), opt(c.icon.clone())],
            )
            .await?;
        }
        Event::MessageSent(m) => {
            if let MessagePurpose::Digest { until, .. } = m.purpose {
                db.exec(
                    "INSERT INTO digests (seq, until_ms, ok, error) VALUES (?1, ?2, ?3, ?4)",
                    vec![
                        seq.into(),
                        ms(until).into(),
                        i64::from(m.ok).into(),
                        opt(m.error.clone()),
                    ],
                )
                .await?;
            }
        }
        // Kept in the log only: nothing asks the index for them.
        Event::Played(_)
        | Event::BlobAdded(_)
        | Event::Started(_)
        | Event::Stopped(_)
        | Event::LogRepaired(_)
        | Event::SettingsChanged(_)
        | Event::Login(_)
        | Event::ImportDone(_)
        | Event::ImportedAudit(_)
        | Event::Unknown { .. } => {}
    }
    Ok(())
}
