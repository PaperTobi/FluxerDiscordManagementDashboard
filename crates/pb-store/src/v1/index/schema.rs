//! The index's tables. The index is derived from the log; a change here bumps [`SCHEMA`] and the index is rebuilt.

/// Bump on any change below.
pub const SCHEMA: i64 = 3;

pub const DDL: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);
CREATE TABLE IF NOT EXISTS sentences (
    seq INTEGER PRIMARY KEY, id TEXT NOT NULL, ts_ms INTEGER NOT NULL, guild TEXT NOT NULL, user TEXT NOT NULL,
    started_ms INTEGER NOT NULL, dur_ms INTEGER NOT NULL, flagged INTEGER NOT NULL, violation INTEGER NOT NULL,
    label TEXT, step INTEGER, audio TEXT, audio_deleted INTEGER NOT NULL DEFAULT 0, data TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS sentences_id ON sentences (id);
CREATE INDEX IF NOT EXISTS sentences_person ON sentences (guild, user, seq);
CREATE INDEX IF NOT EXISTS sentences_violation ON sentences (violation, seq);
CREATE INDEX IF NOT EXISTS sentences_audio ON sentences (audio);
CREATE TABLE IF NOT EXISTS actions (
    seq INTEGER PRIMARY KEY, id TEXT NOT NULL, ts_ms INTEGER NOT NULL, guild TEXT NOT NULL, user TEXT NOT NULL,
    done INTEGER NOT NULL, due_ms INTEGER, undoes TEXT, undone INTEGER NOT NULL DEFAULT 0, data TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS actions_id ON actions (id);
CREATE INDEX IF NOT EXISTS actions_due ON actions (undone, due_ms);
CREATE TABLE IF NOT EXISTS jar (guild TEXT NOT NULL, user TEXT NOT NULL, count INTEGER NOT NULL, PRIMARY KEY (guild, user));
CREATE TABLE IF NOT EXISTS audit (seq INTEGER PRIMARY KEY, ts_ms INTEGER NOT NULL, kind TEXT NOT NULL, v INTEGER NOT NULL, guild TEXT, user TEXT, data TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS audit_kind ON audit (kind, seq);
CREATE INDEX IF NOT EXISTS audit_guild ON audit (guild, seq);
CREATE TABLE IF NOT EXISTS clips (render TEXT PRIMARY KEY, seq INTEGER NOT NULL, added_ms INTEGER NOT NULL, removed INTEGER NOT NULL DEFAULT 0, data TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS people (user TEXT PRIMARY KEY, username TEXT NOT NULL, display_name TEXT, avatar TEXT);
CREATE TABLE IF NOT EXISTS nicks (guild TEXT NOT NULL, user TEXT NOT NULL, nick TEXT, PRIMARY KEY (guild, user));
CREATE TABLE IF NOT EXISTS communities (guild TEXT PRIMARY KEY, name TEXT NOT NULL, icon TEXT);
CREATE TABLE IF NOT EXISTS digests (seq INTEGER PRIMARY KEY, until_ms INTEGER NOT NULL, ok INTEGER NOT NULL, error TEXT);
";
