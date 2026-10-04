//! Version 1.

mod history;
mod old;
mod settings;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use jiff::Timestamp;
use pb_domain::{ActionKind, ActionOutcome, BlobHash, GuildId, Lang, SentenceId, UserId};
use pb_settings::SettingsTree;
use pb_store_api::{
    ActionRecord, Actor, BlobAdded, BlobRole, BlobStore, ClipRecord, Event, EventLog, ImportDone, ImportedAudit,
    JarBaseline, PersonSeen, SecretsFile, SettingsChanged, SettingsFiles, StoreError, Via,
};
use secrecy::SecretString;
use serde::Serialize;

/// Where the old bot's data is.
#[derive(Debug, Clone)]
pub struct Source {
    /// The old `DATA_DIR` (with `bot.db`).
    pub data_dir: PathBuf,
    /// The old image's built-in clips (`CLIPS_DIR`, with `clips.json`), for `builtin.*` clips the settings use.
    pub builtin_clips: Option<PathBuf>,
}

/// Where the import writes.
#[derive(Clone, Copy)]
pub struct Targets<'a> {
    pub log: &'a dyn EventLog,
    pub blobs: &'a dyn BlobStore,
    pub settings: &'a dyn SettingsFiles,
    pub secrets: &'a dyn SecretsFile,
    /// The new `voices/` directory (one folder per voice).
    pub voices_dir: &'a Path,
    /// A scratch directory (the old database is read from a copy there).
    pub scratch: &'a Path,
}

impl std::fmt::Debug for Targets<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Targets")
            .field("voices_dir", &self.voices_dir)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("the event log is not empty: the import only runs into a new data directory")]
    LogNotEmpty,
    #[error("no bot.db in {0}")]
    NoDatabase(PathBuf),
    #[error("reading the old database: {0}")]
    OldDb(String),
    #[error("i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// What was imported.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportReport {
    pub from: String,
    pub sentences: u64,
    pub violations: u64,
    /// Sentences made from a recording whose history row the old bot had already deleted.
    pub from_recordings_only: u64,
    pub recordings: u64,
    pub clips: u64,
    pub settings_rows: u64,
    pub settings_changes: u64,
    pub tracked: u64,
    pub people: u64,
    pub audit: u64,
    pub jar: u64,
    pub pending_mutes: u64,
    pub voices: u64,
    pub secrets: bool,
    pub events: u64,
    pub notes: Vec<String>,
}

impl ImportReport {
    /// A plain-text report for the operator.
    pub fn to_text(&self) -> String {
        let mut s = format!(
            "Imported from {}\n\
             sentences: {} ({} violations, {} only from recordings)\n\
             recordings: {}\nclips: {}\nsettings: {} old rows → {} changes, {} tracked people\n\
             people: {}\naudit entries: {}\nswear jar counts: {}\npending timed mutes: {}\nvoices: {}\nsecrets: {}\n\
             events written: {}\n",
            self.from,
            self.sentences,
            self.violations,
            self.from_recordings_only,
            self.recordings,
            self.clips,
            self.settings_rows,
            self.settings_changes,
            self.tracked,
            self.people,
            self.audit,
            self.jar,
            self.pending_mutes,
            self.voices,
            if self.secrets {
                "carried over"
            } else {
                "not carried over"
            },
            self.events,
        );
        if !self.notes.is_empty() {
            s.push_str("\nNotes:\n");
            for n in &self.notes {
                s.push_str("- ");
                s.push_str(n);
                s.push('\n');
            }
        }
        s
    }
}

/// Whether a data directory holds an old bot's data that has not been imported (`bot.db` and an empty log).
pub fn pending(data_dir: &Path, log: &dyn EventLog) -> bool {
    data_dir.join("bot.db").is_file() && log.head().is_none()
}

fn importer() -> Actor {
    Actor {
        user: None,
        name: Some("import".into()),
        via: Via::Import,
    }
}

fn wav_ms(bytes: &[u8]) -> Option<u32> {
    u32::try_from(pb_audio::wav_layout(bytes)?.millis()).ok()
}

/// Stores a file as a blob; returns its hash and the `blob.added` event.
async fn put(blobs: &dyn BlobStore, bytes: Vec<u8>, role: BlobRole) -> Result<(BlobHash, Event), StoreError> {
    let info = blobs.put(Bytes::from(bytes)).await?;
    Ok((
        info.hash,
        Event::BlobAdded(BlobAdded {
            hash: info.hash,
            size: info.size,
            media_type: "audio/wav".into(),
            role,
        }),
    ))
}

/// `builtin.<stem>` clips the old settings refer to.
fn builtin_refs(old: &old::OldDb) -> Vec<String> {
    let mut out = Vec::new();
    for r in &old.settings {
        if r.text(2).as_deref() != Some("clips") {
            continue;
        }
        let v: serde_json::Value = r
            .text(3)
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        let mut stack = vec![v];
        while let Some(v) = stack.pop() {
            match v {
                serde_json::Value::String(s) if s.starts_with("builtin.") && !out.contains(&s) => out.push(s),
                serde_json::Value::Array(a) => stack.extend(a),
                serde_json::Value::Object(o) => stack.extend(o.into_values()),
                _ => {}
            }
        }
    }
    out
}

/// Imports the old data directory. `tree` is the current settings (loaded from the files); it is changed and the
/// files are written.
pub async fn import(src: &Source, t: Targets<'_>, tree: &mut SettingsTree) -> Result<ImportReport, ImportError> {
    if t.log.head().is_some() {
        return Err(ImportError::LogNotEmpty);
    }
    let db_file = src.data_dir.join("bot.db");
    if !db_file.is_file() {
        return Err(ImportError::NoDatabase(src.data_dir.clone()));
    }
    let old = old::read(&db_file, t.scratch).await?;
    let now = Timestamp::now();
    let mut report = ImportReport {
        from: src.data_dir.display().to_string(),
        ..ImportReport::default()
    };
    let mut events: Vec<(Timestamp, Event)> = Vec::new();

    // Clips: uploads, and the built-in ones the settings use.
    let mut clip_hash: BTreeMap<String, BlobHash> = BTreeMap::new();
    for r in &old.clips {
        let (Some(id), Some(file)) = (r.text(0), r.text(2)) else {
            continue;
        };
        let path = src.data_dir.join("clips").join(&file);
        let Ok(bytes) = std::fs::read(&path) else {
            report
                .notes
                .push(format!("clip {id} ({file}) is missing and was not imported"));
            continue;
        };
        let dur_ms = wav_ms(&bytes).unwrap_or(0);
        let (hash, added) = put(t.blobs, bytes, BlobRole::Render).await?;
        let at = r.real(7).map_or(now, history::ts);
        let uploader = Actor {
            user: r.text(6).and_then(|u| u.parse().ok()),
            name: None,
            via: Via::Import,
        };
        events.push((at, added));
        events.push((
            at,
            Event::ClipSaved(Box::new(ClipRecord {
                render: hash,
                original: hash,
                name: r.text(1).unwrap_or_else(|| id.clone()),
                lang: None,
                transcript: r.text(3).filter(|t| !t.is_empty()),
                dur_ms,
                self_check: history::scores(r.text(8).as_deref()),
                heard_language: None,
                added_by: uploader.clone(),
                by: uploader,
            })),
        ));
        clip_hash.insert(id, hash);
        report.clips += 1;
    }
    let refs = builtin_refs(&old);
    if !refs.is_empty() {
        let texts: BTreeMap<String, String> = src
            .builtin_clips
            .as_ref()
            .and_then(|d| std::fs::read_to_string(d.join("clips.json")).ok())
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("clips").and_then(|c| c.as_array()).cloned())
            .into_iter()
            .flatten()
            .filter_map(|c| Some((c.get("file")?.as_str()?.to_owned(), c.get("text")?.as_str()?.to_owned())))
            .collect();
        for id in refs {
            let stem = id.trim_start_matches("builtin.");
            let file = format!("{stem}.wav");
            let bytes = src
                .builtin_clips
                .as_ref()
                .and_then(|d| std::fs::read(d.join(&file)).ok());
            let Some(bytes) = bytes else {
                report.notes.push(format!(
                    "built-in clip {id} was not found (give the old clips folder) and is left out"
                ));
                continue;
            };
            let dur_ms = wav_ms(&bytes).unwrap_or(0);
            let (hash, added) = put(t.blobs, bytes, BlobRole::Render).await?;
            events.push((now, added));
            let text = texts.get(&file).cloned();
            events.push((
                now,
                Event::ClipSaved(Box::new(ClipRecord {
                    render: hash,
                    original: hash,
                    name: text.clone().unwrap_or_else(|| stem.replace('_', " ")),
                    lang: "en".parse::<Lang>().ok(),
                    transcript: text,
                    dur_ms,
                    self_check: None,
                    heard_language: None,
                    added_by: importer(),
                    by: importer(),
                })),
            ));
            clip_hash.insert(id, hash);
            report.clips += 1;
        }
    }

    // Recordings.
    let evidence_root = src.data_dir.join("evidence");
    let mut audio: BTreeMap<String, BlobHash> = BTreeMap::new();
    for r in &old.evidence {
        let (Some(id), Some(rel)) = (r.text(0), r.text(8)) else {
            continue;
        };
        let path = src.data_dir.join(&rel);
        let inside = path
            .canonicalize()
            .ok()
            .zip(evidence_root.canonicalize().ok())
            .is_some_and(|(p, root)| p.starts_with(root));
        let bytes = if inside { std::fs::read(&path).ok() } else { None };
        let Some(bytes) = bytes else {
            report
                .notes
                .push(format!("recording {id} ({rel}) is missing and was not imported"));
            continue;
        };
        let (hash, added) = put(t.blobs, bytes, BlobRole::Recording).await?;
        events.push((r.real(1).map_or(now, history::ts), added));
        audio.insert(id, hash);
        report.recordings += 1;
    }

    // Settings and tracked people → TOML.
    let conv = settings::convert(&old, tree, &clip_hash);
    report.settings_rows = conv.rows;
    report.settings_changes = conv.changes.len() as u64;
    report.tracked = old.tracked.len() as u64;
    report.notes.extend(conv.notes);
    t.settings.write(&conv.changes).await?;
    for c in conv.changes {
        events.push((
            now,
            Event::SettingsChanged(Box::new(SettingsChanged {
                change: c,
                by: importer(),
            })),
        ));
    }

    // History.
    let window = tree
        .effective(None, None)
        .violation_window
        .value
        .value()
        .map(|d| d.get().secs());
    let (sentences, orphans, notes) = history::convert(&old, &audio, window);
    report.notes.extend(notes);
    report.from_recordings_only = orphans;
    if orphans > 0 {
        report.notes.push(format!(
            "{orphans} recording(s) had no history row any more; they are recorded as warned violations with only the \
             score of their detection type (the old bot did not keep more)"
        ));
    }
    for (at, s) in sentences {
        report.sentences += 1;
        report.violations += u64::from(s.decision.is_violation());
        events.push((at, Event::Sentence(Box::new(s))));
    }

    // People.
    for r in &old.people {
        let Some(user) = r.text(0).and_then(|u| u.parse::<UserId>().ok()) else {
            continue;
        };
        report.people += 1;
        events.push((
            r.real(4).map_or(now, history::ts),
            Event::PersonSeen(PersonSeen {
                user,
                guild: None,
                username: r.text(1).unwrap_or_else(|| user.to_string()),
                display_name: r.text(2),
                nick: None,
                avatar: r.text(3),
            }),
        ));
    }

    // The old audit log, as recorded.
    for r in &old.audit {
        report.audit += 1;
        let scope = r.text(4).unwrap_or_default();
        let id = r.text(5).unwrap_or_default();
        let (guild, user) = match scope.as_str() {
            "server" => (id.parse::<GuildId>().ok(), None),
            "person" => id
                .split_once(':')
                .map_or((None, None), |(g, u)| (g.parse().ok(), u.parse().ok())),
            _ => (None, None),
        };
        let raw = |i: usize| {
            r.text(i)
                .map(|t| serde_json::from_str(&t).unwrap_or(serde_json::Value::String(t)))
        };
        let at = r.real(0).map_or(now, history::ts);
        events.push((
            at,
            Event::ImportedAudit(Box::new(ImportedAudit {
                at,
                actor: Actor {
                    user: r.text(1).and_then(|u| u.parse().ok()),
                    name: r.text(2),
                    via: Via::Import,
                },
                source: r.text(3).unwrap_or_default(),
                guild,
                user,
                key: r.text(6).unwrap_or_default(),
                before: raw(7),
                after: raw(8),
            })),
        ));
    }

    // Swear jar counts.
    for r in &old.jar {
        let (Some(guild), Some(user), Some(count)) = (
            r.text(0).and_then(|g| g.parse::<GuildId>().ok()),
            r.text(1).and_then(|u| u.parse::<UserId>().ok()),
            r.int(2).and_then(|c| u64::try_from(c).ok()),
        ) else {
            continue;
        };
        if count > 0 {
            report.jar += 1;
            events.push((now, Event::JarBaseline(JarBaseline { guild, user, count })));
        }
    }

    // Timed mutes still to lift.
    for r in &old.actions {
        if r.text(5).as_deref() != Some("pending") || r.text(4).as_deref() != Some("unmute") {
            continue;
        }
        let (Some(guild), Some(user)) = (
            r.text(2).and_then(|g| g.parse().ok()),
            r.text(3).and_then(|u| u.parse().ok()),
        ) else {
            continue;
        };
        report.pending_mutes += 1;
        events.push((
            r.real(0).map_or(now, history::ts),
            Event::Action(Box::new(ActionRecord {
                id: SentenceId::new(),
                guild,
                user,
                kind: ActionKind::Mute,
                secs: None,
                sentence: None,
                step: None,
                outcome: ActionOutcome::Done,
                undo_at: r.real(1).map(history::ts),
                undoes: None,
                retry_at: None,
            })),
        ));
    }

    // Installed voices: hard-linked (or copied) into voices/<id>/.
    let old_voices = src.data_dir.join("voices");
    if old_voices.is_dir() {
        for e in std::fs::read_dir(&old_voices)?.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let Some(id) = name.strip_suffix(".onnx") else { continue };
            let dir = t.voices_dir.join(id);
            std::fs::create_dir_all(&dir)?;
            for f in [format!("{id}.onnx"), format!("{id}.onnx.json")] {
                let from = old_voices.join(&f);
                let to = dir.join(&f);
                if from.exists() && !to.exists() && std::fs::hard_link(&from, &to).is_err() {
                    std::fs::copy(&from, &to)?;
                }
            }
            report.voices += 1;
        }
    }

    // Secrets (unless the new directory already has a token).
    if let Ok(text) = std::fs::read_to_string(src.data_dir.join("secrets.json")) {
        let old: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
        let mut s = t.secrets.load().await?;
        if s.bot_token.is_none() {
            let get = |k: &str| {
                old.get(k)
                    .and_then(|v| v.as_str())
                    .filter(|v| !v.is_empty())
                    .map(|v| SecretString::from(v.to_owned()))
            };
            s.bot_token = get("bot_token");
            s.client_secret = s.client_secret.or_else(|| get("client_secret"));
            s.cookie_key = s.cookie_key.or_else(|| get("session_key"));
            if old.get("setup_done").and_then(serde_json::Value::as_bool) == Some(true) {
                s.setup.done = true;
                s.setup.owner = old
                    .get("setup_owner")
                    .and_then(|v| v.as_str())
                    .and_then(|u| u.parse().ok());
                s.setup.finished_at = old
                    .get("setup_finished_at")
                    .and_then(serde_json::Value::as_f64)
                    .map(history::ts);
            }
            t.secrets.save(&s).await?;
            report.secrets = true;
        } else {
            report
                .notes
                .push("secrets.json was not carried over: the new data directory already has a bot token".into());
        }
    }
    report.notes.push(
        "login sessions and the speech cache were not carried over (log in again; speech is rendered anew)".into(),
    );

    // Everything into the log, oldest first, in one batch (all or nothing). This append is the commit point: the
    // files written above (blobs by content, settings, voices, secrets) are written the same again if the import
    // stops before it and runs anew.
    events.sort_by_key(|(at, _)| *at);
    let done = ImportDone {
        from: report.from.clone(),
        sentences: report.sentences,
        violations: report.violations,
        recordings: report.recordings,
        clips: report.clips,
        settings: report.settings_changes,
        audit: report.audit,
        notes: report.notes.clone(),
    };
    events.push((Timestamp::now(), Event::ImportDone(done)));
    let new: Vec<_> = events.iter().filter_map(|(at, e)| e.to_new(Some(*at))).collect();
    report.events = new.len() as u64;
    t.log.append(new).await?;
    Ok(report)
}
