//! Reading the tables.

use std::collections::BTreeMap;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use pb_domain::{GuildId, Label, SentenceId, UserId};
use pb_store_api::{
    AUDIT_KINDS, ActionRecord, AuditFilter, AuditRow, ClipRecord, ClipRow, CommunitySeen, Cursor, DayRow, DigestRow,
    Event, JarRow, LastDigest, Page, PersonName, SentenceFilter, SentenceKind, SentenceRecord, SentenceRow, StoreError,
    VoiceRecord, VoiceRow,
};
use turso::Value;

use super::db::{Db, int, text};

fn bad(what: &str) -> StoreError {
    StoreError::Index(format!("unreadable {what} row"))
}

fn parse<T: std::str::FromStr>(v: &Value, what: &str) -> Result<T, StoreError> {
    text(v).and_then(|s| s.parse().ok()).ok_or_else(|| bad(what))
}

fn decode<T: serde::de::DeserializeOwned>(v: &Value, what: &str) -> Result<T, StoreError> {
    let s = text(v).ok_or_else(|| bad(what))?;
    serde_json::from_str(&s).map_err(|e| StoreError::Index(format!("{what}: {e}")))
}

fn seq(v: &Value) -> u64 {
    int(v).and_then(|i| u64::try_from(i).ok()).unwrap_or(0)
}

fn ts(v: &Value) -> Timestamp {
    int(v)
        .and_then(|ms| Timestamp::from_millisecond(ms).ok())
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

fn cursor_seq(c: Option<Cursor>) -> i64 {
    c.map_or(i64::MAX, |c| i64::try_from(c.0).unwrap_or(i64::MAX))
}

/// Builds a page from `limit + 1` rows.
fn page<T>(mut items: Vec<T>, limit: u32, seq_of: impl Fn(&T) -> u64) -> Page<T> {
    let limit = limit.max(1) as usize;
    let next = if items.len() > limit {
        items.truncate(limit);
        items.last().map(|i| Cursor(seq_of(i)))
    } else {
        None
    };
    Page { items, next }
}

struct Where {
    clauses: Vec<String>,
    params: Vec<Value>,
}

impl Where {
    fn new() -> Where {
        Where {
            clauses: Vec::new(),
            params: Vec::new(),
        }
    }

    fn add(&mut self, clause: &str, value: impl Into<Value>) {
        self.params.push(value.into());
        self.clauses
            .push(clause.replace('?', &format!("?{}", self.params.len())));
    }

    /// `column IN (values)` (an empty list matches nothing).
    fn any_of(&mut self, column: &str, values: impl IntoIterator<Item = impl Into<Value>>) {
        let mut marks = Vec::new();
        for v in values {
            self.params.push(v.into());
            marks.push(format!("?{}", self.params.len()));
        }
        if marks.is_empty() {
            self.clauses.push("0".to_owned());
        } else {
            self.clauses.push(format!("{column} IN ({})", marks.join(", ")));
        }
    }

    fn fixed(&mut self, clause: &str) {
        self.clauses.push(clause.to_owned());
    }

    fn sql(&self) -> String {
        if self.clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.clauses.join(" AND "))
        }
    }

    fn limit(mut self, n: u32) -> (String, Vec<Value>) {
        self.params.push(i64::from(n.max(1)).saturating_add(1).into());
        let i = self.params.len();
        (format!("{} ORDER BY seq DESC LIMIT ?{i}", self.sql()), self.params)
    }
}

pub async fn sentences(
    db: &Db,
    f: SentenceFilter,
    cursor: Option<Cursor>,
    limit: u32,
) -> Result<Page<SentenceRow>, StoreError> {
    let mut w = Where::new();
    w.add("seq < ?", cursor_seq(cursor));
    if let Some(gs) = f.guilds {
        w.any_of("guild", gs.iter().map(ToString::to_string));
    }
    if let Some(u) = f.user {
        w.add("user = ?", u.to_string());
    }
    if let Some(t) = f.since {
        w.add("started_ms >= ?", t.as_millisecond());
    }
    if let Some(t) = f.until {
        w.add("started_ms < ?", t.as_millisecond());
    }
    match f.kind {
        SentenceKind::All => {}
        SentenceKind::Flagged => w.fixed("flagged = 1"),
        SentenceKind::Violations => w.fixed("violation = 1"),
        SentenceKind::WithAudio => w.fixed("audio IS NOT NULL AND audio_deleted = 0"),
    }
    if let Some(l) = f.label {
        w.add("label = ?", l.key().to_owned());
    }
    let (clause, params) = w.limit(limit);
    let rows = db
        .rows(
            &format!("SELECT seq, data, audio_deleted FROM sentences{clause}"),
            params,
        )
        .await?;
    let items = rows
        .iter()
        .map(|r| {
            Ok(SentenceRow {
                seq: seq(&r[0]),
                record: decode::<SentenceRecord>(&r[1], "sentence")?,
                audio_deleted: int(&r[2]) == Some(1),
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok(page(items, limit, |r| r.seq))
}

pub async fn sentence(db: &Db, id: SentenceId) -> Result<Option<SentenceRow>, StoreError> {
    let rows = db
        .rows(
            "SELECT seq, data, audio_deleted FROM sentences WHERE id = ?1",
            vec![id.to_string().into()],
        )
        .await?;
    rows.first()
        .map(|r| {
            Ok(SentenceRow {
                seq: seq(&r[0]),
                record: decode(&r[1], "sentence")?,
                audio_deleted: int(&r[2]) == Some(1),
            })
        })
        .transpose()
}

pub async fn days(
    db: &Db,
    guild: GuildId,
    user: UserId,
    from: Date,
    until: Date,
    tz: String,
) -> Result<Vec<DayRow>, StoreError> {
    let zone = TimeZone::get(&tz).map_err(|e| StoreError::Invalid(format!("time zone {tz:?}: {e}")))?;
    let start = from
        .to_zoned(zone.clone())
        .map_err(|e| StoreError::Invalid(e.to_string()))?
        .timestamp();
    let end_day = until.tomorrow().map_err(|e| StoreError::Invalid(e.to_string()))?;
    let end = end_day
        .to_zoned(zone.clone())
        .map_err(|e| StoreError::Invalid(e.to_string()))?
        .timestamp();
    let rows = db
        .rows(
            "SELECT started_ms, dur_ms, flagged, violation FROM sentences \
             WHERE guild = ?1 AND user = ?2 AND started_ms >= ?3 AND started_ms < ?4",
            vec![
                guild.to_string().into(),
                user.to_string().into(),
                start.as_millisecond().into(),
                end.as_millisecond().into(),
            ],
        )
        .await?;
    let mut by_day: BTreeMap<Date, DayRow> = BTreeMap::new();
    let mut d = from;
    while d <= until {
        by_day.insert(
            d,
            DayRow {
                day: d,
                sentences: 0,
                flagged: 0,
                violations: 0,
                speech_ms: 0,
            },
        );
        d = match d.tomorrow() {
            Ok(n) => n,
            Err(_) => break,
        };
    }
    for r in rows {
        let day = ts(&r[0]).to_zoned(zone.clone()).date();
        if let Some(row) = by_day.get_mut(&day) {
            row.sentences += 1;
            row.speech_ms += int(&r[1]).and_then(|v| u64::try_from(v).ok()).unwrap_or(0);
            row.flagged += u32::from(int(&r[2]) == Some(1));
            row.violations += u32::from(int(&r[3]) == Some(1));
        }
    }
    Ok(by_day.into_values().collect())
}

pub async fn jar(db: &Db, guild: Option<GuildId>) -> Result<Vec<JarRow>, StoreError> {
    let (sql, params) = match guild {
        Some(g) => (
            "SELECT guild, user, count FROM jar WHERE count > 0 AND guild = ?1 ORDER BY count DESC, user",
            vec![g.to_string().into()],
        ),
        None => (
            "SELECT guild, user, count FROM jar WHERE count > 0 ORDER BY count DESC, user",
            Vec::new(),
        ),
    };
    db.rows(sql, params)
        .await?
        .iter()
        .map(|r| {
            Ok(JarRow {
                guild: parse(&r[0], "jar")?,
                user: parse(&r[1], "jar")?,
                count: seq(&r[2]),
            })
        })
        .collect()
}

pub async fn violation_times(db: &Db) -> Result<Vec<(GuildId, UserId, Timestamp)>, StoreError> {
    db.rows(
        "SELECT guild, user, ts_ms FROM sentences WHERE violation = 1 ORDER BY seq",
        Vec::new(),
    )
    .await?
    .iter()
    .map(|r| Ok((parse(&r[0], "violation")?, parse(&r[1], "violation")?, ts(&r[2]))))
    .collect()
}

pub async fn pending_undos(db: &Db) -> Result<Vec<ActionRecord>, StoreError> {
    db.rows(
        "SELECT data, due_ms FROM actions WHERE undone = 0 AND undoes IS NULL AND due_ms IS NOT NULL ORDER BY due_ms",
        Vec::new(),
    )
    .await?
    .iter()
    .map(|r| {
        let mut a: ActionRecord = decode(&r[0], "action")?;
        a.undo_at = Some(ts(&r[1]));
        Ok(a)
    })
    .collect()
}

pub async fn audit(db: &Db, f: AuditFilter, cursor: Option<Cursor>, limit: u32) -> Result<Page<AuditRow>, StoreError> {
    let mut w = Where::new();
    w.add("seq < ?", cursor_seq(cursor));
    if let Some(gs) = f.guilds {
        w.any_of("guild", gs.iter().map(ToString::to_string));
    }
    if let Some(u) = f.user {
        w.add("user = ?", u.to_string());
    }
    if f.kinds.is_empty() {
        w.any_of("kind", AUDIT_KINDS.iter().copied());
    } else {
        w.any_of("kind", f.kinds);
    }
    let (clause, params) = w.limit(limit);
    let items = db
        .rows(&format!("SELECT seq, ts_ms, kind, v, data FROM audit{clause}"), params)
        .await?
        .iter()
        .map(|r| {
            let kind = text(&r[2]).unwrap_or_default();
            let v = int(&r[3]).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
            let data: serde_json::Value = decode(&r[4], "audit")?;
            let event =
                Event::decode(&kind, v, &data).map_err(|e| StoreError::Index(format!("audit {kind} v{v}: {e}")))?;
            Ok(AuditRow {
                seq: seq(&r[0]),
                ts: ts(&r[1]),
                event,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
    Ok(page(items, limit, |r| r.seq))
}

pub async fn clips(db: &Db) -> Result<Vec<ClipRow>, StoreError> {
    db.rows(
        "SELECT seq, added_ms, data FROM clips WHERE removed = 0 ORDER BY added_ms DESC",
        Vec::new(),
    )
    .await?
    .iter()
    .map(|r| {
        Ok(ClipRow {
            seq: seq(&r[0]),
            added: ts(&r[1]),
            record: decode::<ClipRecord>(&r[2], "clip")?,
        })
    })
    .collect()
}

pub async fn voices(db: &Db) -> Result<Vec<VoiceRow>, StoreError> {
    db.rows(
        "SELECT seq, added_ms, data FROM voices WHERE removed = 0 ORDER BY added_ms, model, id",
        Vec::new(),
    )
    .await?
    .iter()
    .map(|r| {
        Ok(VoiceRow {
            seq: seq(&r[0]),
            added: ts(&r[1]),
            record: decode::<VoiceRecord>(&r[2], "voice")?,
        })
    })
    .collect()
}

pub async fn people(db: &Db, users: Vec<UserId>, guild: Option<GuildId>) -> Result<Vec<PersonName>, StoreError> {
    let mut out = Vec::new();
    for u in users {
        let rows = db
            .rows(
                "SELECT p.username, p.display_name, p.avatar, n.nick FROM people p \
                 LEFT JOIN nicks n ON n.user = p.user AND n.guild = ?2 WHERE p.user = ?1",
                vec![
                    u.to_string().into(),
                    guild.map_or(Value::Null, |g| g.to_string().into()),
                ],
            )
            .await?;
        if let Some(r) = rows.first() {
            out.push(PersonName {
                user: u,
                username: text(&r[0]).unwrap_or_default(),
                display_name: text(&r[1]),
                avatar: text(&r[2]),
                nick: text(&r[3]),
            });
        }
    }
    Ok(out)
}

pub async fn communities(db: &Db) -> Result<Vec<CommunitySeen>, StoreError> {
    db.rows("SELECT guild, name, icon FROM communities ORDER BY name", Vec::new())
        .await?
        .iter()
        .map(|r| {
            Ok(CommunitySeen {
                guild: parse(&r[0], "community")?,
                name: text(&r[1]).unwrap_or_default(),
                icon: text(&r[2]),
            })
        })
        .collect()
}

pub async fn last_digest(db: &Db) -> Result<LastDigest, StoreError> {
    let rows = db
        .rows(
            "SELECT (SELECT MAX(until_ms) FROM digests), (SELECT MAX(until_ms) FROM digests WHERE ok = 1), \
             (SELECT error FROM digests ORDER BY seq DESC LIMIT 1)",
            Vec::new(),
        )
        .await?;
    let at = |i: usize| {
        rows.first()
            .and_then(|r| int(&r[i]))
            .and_then(|ms| Timestamp::from_millisecond(ms).ok())
    };
    Ok(LastDigest {
        tried: at(0),
        sent: at(1),
        error: rows.first().and_then(|r| text(&r[2])),
    })
}

pub async fn digest(db: &Db, from: Timestamp, until: Timestamp) -> Result<Vec<DigestRow>, StoreError> {
    let range = || vec![Value::from(from.as_millisecond()), Value::from(until.as_millisecond())];
    let rows = db
        .rows(
            "SELECT guild, user, label, step FROM sentences WHERE violation = 1 AND ts_ms >= ?1 AND ts_ms < ?2",
            range(),
        )
        .await?;
    /// violations, per label, highest step, actions
    type Tally = (u32, BTreeMap<String, u32>, u32, u32);
    let mut per: BTreeMap<(GuildId, UserId), Tally> = BTreeMap::new();
    for r in &rows {
        let key = (parse(&r[0], "digest")?, parse(&r[1], "digest")?);
        let e = per.entry(key).or_default();
        e.0 += 1;
        *e.1.entry(text(&r[2]).unwrap_or_default()).or_default() += 1;
        e.2 = e.2.max(int(&r[3]).and_then(|s| u32::try_from(s).ok()).unwrap_or(0));
    }
    let actions = db
        .rows(
            "SELECT guild, user, COUNT(*) FROM actions WHERE done = 1 AND undoes IS NULL AND ts_ms >= ?1 AND ts_ms < ?2 GROUP BY guild, user",
            range(),
        )
        .await?;
    for r in &actions {
        let key = (parse(&r[0], "digest")?, parse(&r[1], "digest")?);
        if let Some(e) = per.get_mut(&key) {
            e.3 = int(&r[2]).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
        }
    }
    let mut out: Vec<DigestRow> = per
        .into_iter()
        .map(|((guild, user), (violations, labels, max_step, actions))| {
            let top = labels
                .iter()
                .max_by_key(|(_, n)| **n)
                .and_then(|(l, _)| l.parse::<Label>().ok())
                .unwrap_or(Label::Profanity);
            DigestRow {
                guild,
                user,
                violations,
                top_label: top,
                max_step,
                actions,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.violations
            .cmp(&a.violations)
            .then(a.guild.cmp(&b.guild))
            .then(a.user.cmp(&b.user))
    });
    Ok(out)
}
