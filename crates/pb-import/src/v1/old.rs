//! Reading the old `bot.db` (a SQLite file; Turso reads it).

use std::path::Path;

use turso::Value;

use super::ImportError;

/// A row as the old bot wrote it.
#[derive(Debug, Clone, Default)]
pub struct Row(pub Vec<Value>);

impl Row {
    pub fn text(&self, i: usize) -> Option<String> {
        match self.0.get(i)? {
            Value::Text(s) => Some(s.clone()),
            Value::Integer(n) => Some(n.to_string()),
            Value::Real(f) => Some(f.to_string()),
            _ => None,
        }
    }

    pub fn real(&self, i: usize) -> Option<f64> {
        match self.0.get(i)? {
            Value::Real(f) => Some(*f),
            Value::Integer(n) => Some(*n as f64),
            Value::Text(s) => s.parse().ok(),
            _ => None,
        }
    }

    pub fn int(&self, i: usize) -> Option<i64> {
        match self.0.get(i)? {
            Value::Integer(n) => Some(*n),
            Value::Real(f) => Some(*f as i64),
            Value::Text(s) => s.parse().ok(),
            _ => None,
        }
    }
}

/// Every table the import reads.
#[derive(Debug, Default)]
pub struct OldDb {
    pub settings: Vec<Row>,
    pub tracked: Vec<Row>,
    pub people: Vec<Row>,
    pub history: Vec<Row>,
    pub violations: Vec<Row>,
    pub evidence: Vec<Row>,
    pub clips: Vec<Row>,
    pub audit: Vec<Row>,
    pub jar: Vec<Row>,
    pub actions: Vec<Row>,
}

async fn all(conn: &turso::Connection, sql: &str) -> Result<Vec<Row>, ImportError> {
    let mut rows = conn
        .query(sql, ())
        .await
        .map_err(|e| ImportError::OldDb(format!("{sql}: {e}")))?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(|e| ImportError::OldDb(e.to_string()))? {
        let mut cols = Vec::with_capacity(row.column_count());
        for i in 0..row.column_count() {
            cols.push(row.get_value(i).map_err(|e| ImportError::OldDb(e.to_string()))?);
        }
        out.push(Row(cols));
    }
    Ok(out)
}

/// Reads a copy of the old database (the original, and its WAL, stay untouched).
pub async fn read(db_file: &Path, scratch: &Path) -> Result<OldDb, ImportError> {
    std::fs::create_dir_all(scratch)?;
    let copy = scratch.join("old-bot.db");
    std::fs::copy(db_file, &copy)?;
    // The old bot ran SQLite in WAL mode: committed rows may still be in the -wal file.
    let wal = db_file.with_file_name(format!(
        "{}-wal",
        db_file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
    ));
    if wal.exists() {
        std::fs::copy(&wal, scratch.join("old-bot.db-wal"))?;
    }
    let path = copy
        .to_str()
        .ok_or_else(|| ImportError::OldDb("the scratch path is not UTF-8".into()))?;
    let db = turso::Builder::new_local(path)
        .build()
        .await
        .map_err(|e| ImportError::OldDb(e.to_string()))?;
    let conn = db.connect().map_err(|e| ImportError::OldDb(e.to_string()))?;
    let version = all(&conn, "PRAGMA user_version")
        .await?
        .first()
        .and_then(|r| r.int(0))
        .unwrap_or(0);
    if version != 1 {
        return Err(ImportError::OldDb(format!(
            "bot.db has schema version {version}; this importer reads version 1"
        )));
    }
    let out = OldDb {
        settings: all(&conn, "SELECT scope, scope_id, key, value, updated_by, updated_at FROM settings ORDER BY scope, scope_id, key").await?,
        tracked: all(&conn, "SELECT guild_id, user_id, added_by, added_at FROM tracked ORDER BY added_at").await?,
        people: all(&conn, "SELECT user_id, username, display_name, avatar, updated_at FROM people").await?,
        history: all(&conn, "SELECT ts, guild_id, channel_id, user_id, clip_uid, dur_s, scores, flagged, decision, reason, lang, step FROM history ORDER BY ts, id").await?,
        violations: all(&conn, "SELECT ts, guild_id, channel_id, user_id, label, score, step, action, played, clip_uid, evidence_id FROM violations ORDER BY ts, id").await?,
        evidence: all(&conn, "SELECT id, ts, guild_id, channel_id, user_id, label, score, dur_s, path, size FROM evidence ORDER BY ts").await?,
        clips: all(&conn, "SELECT id, name, file, text, dur_s, kind, created_by, created_at, scores FROM clips ORDER BY created_at").await?,
        audit: all(&conn, "SELECT ts, actor_id, actor_name, source, scope, scope_id, key, before, after FROM audit ORDER BY ts, id").await?,
        jar: all(&conn, "SELECT guild_id, user_id, count, since FROM jar").await?,
        actions: all(&conn, "SELECT created, due, guild_id, user_id, kind, state, attempts, last_error FROM actions ORDER BY due").await?,
    };
    drop(conn);
    drop(db);
    let _ = std::fs::remove_file(&copy);
    let _ = std::fs::remove_file(scratch.join("old-bot.db-wal"));
    Ok(out)
}
