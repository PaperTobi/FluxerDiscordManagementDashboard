//! The settings files, `secrets.toml` and `sessions.json`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use pb_domain::GuildId;
use pb_settings::{
    Change, FileError, SettingsTree, apply_to_document, file_of, new_document, parse_global, parse_server,
};
use pb_store_api::{
    ApiConfig, ApiFile, Secrets, SecretsFile, SessionRecord, SessionsFile, SettingsFiles, SetupState, StoreError,
};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use super::fsutil::write_atomic;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, StoreError> + Send + 'static,
) -> Result<T, StoreError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| StoreError::Io(e.to_string()))?
}

fn read_optional(path: &Path) -> Result<Option<String>, StoreError> {
    match fs::read_to_string(path) {
        Ok(t) => Ok(Some(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::Io(format!("{}: {e}", path.display()))),
    }
}

// ------------------------------------------------------------------------------------------------ settings

/// `settings/global.toml` and `settings/servers/<community>.toml`.
#[derive(Debug, Clone)]
pub struct FsSettingsFiles {
    dir: PathBuf,
}

const GLOBAL_HEADER: &str = "Global settings of the profanity watch bot. Edited by the web UI and by hand; comments stay.\n\
Every setting is optional: a missing one uses the built-in default. See docs/settings.md.";

fn server_header(guild: GuildId) -> String {
    format!(
        "Settings of community {guild} (they override the global ones), its voice lines and its people.\n\
         Edited by the web UI, chat commands and by hand; comments stay."
    )
}

impl FsSettingsFiles {
    pub fn new(settings_dir: &Path) -> FsSettingsFiles {
        FsSettingsFiles {
            dir: settings_dir.to_owned(),
        }
    }

    fn path(&self, guild: Option<GuildId>) -> PathBuf {
        match guild {
            None => self.dir.join("global.toml"),
            Some(g) => self.dir.join("servers").join(format!("{g}.toml")),
        }
    }

    fn rel(&self, guild: Option<GuildId>) -> String {
        match guild {
            None => "settings/global.toml".into(),
            Some(g) => format!("settings/servers/{g}.toml"),
        }
    }

    fn load_sync(&self) -> Result<(SettingsTree, Vec<FileError>), StoreError> {
        let mut tree = SettingsTree::default();
        let mut problems = Vec::new();
        // A file that cannot be read (no permission, not UTF-8, a directory) is reported and left out.
        let unreadable = |file: String, e: &dyn std::fmt::Display| FileError {
            file,
            message: e.to_string(),
            span: None,
        };
        match read_optional(&self.path(None)) {
            Ok(Some(text)) => match parse_global(&self.rel(None), &text) {
                Ok(g) => tree.global = g,
                Err(e) => problems.push(e),
            },
            Ok(None) => {}
            Err(e) => problems.push(unreadable(self.rel(None), &e)),
        }
        let servers = self.dir.join("servers");
        if servers.exists() {
            let mut entries: Vec<_> = fs::read_dir(&servers)?.flatten().collect();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for e in entries {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(stem) = name.strip_suffix(".toml") else {
                    continue;
                };
                let Ok(guild) = stem.parse::<GuildId>() else {
                    problems.push(FileError {
                        file: format!("settings/servers/{name}"),
                        message: "the file name must be a community ID, like 123456789012345678.toml".into(),
                        span: None,
                    });
                    continue;
                };
                let text = match fs::read_to_string(e.path()) {
                    Ok(t) => t,
                    Err(err) => {
                        problems.push(unreadable(self.rel(Some(guild)), &err));
                        continue;
                    }
                };
                match parse_server(&self.rel(Some(guild)), &text) {
                    Ok(s) => {
                        tree.servers.insert(guild, s);
                    }
                    Err(err) => problems.push(err),
                }
            }
        }
        Ok((tree, problems))
    }

    fn write_sync(&self, changes: &[Change]) -> Result<(), StoreError> {
        let mut by_file: BTreeMap<Option<GuildId>, Vec<&Change>> = BTreeMap::new();
        for c in changes {
            by_file.entry(file_of(c)).or_default().push(c);
        }
        // Every file is edited and checked first, so a refused edit leaves all of them as they were.
        let mut edited = Vec::with_capacity(by_file.len());
        for (guild, changes) in by_file {
            let path = self.path(guild);
            let mut doc = match read_optional(&path)? {
                Some(text) => text.parse::<toml_edit::DocumentMut>().map_err(|e| {
                    StoreError::Invalid(format!(
                        "{} cannot be edited, it is not valid TOML: {e}",
                        self.rel(guild)
                    ))
                })?,
                None => new_document(&guild.map_or_else(|| GLOBAL_HEADER.to_owned(), server_header)),
            };
            for c in changes {
                apply_to_document(&mut doc, c);
            }
            let text = doc.to_string();
            // Never write a file the bot could not read back.
            let check = match guild {
                None => parse_global(&self.rel(guild), &text).map(|_| ()),
                Some(_) => parse_server(&self.rel(guild), &text).map(|_| ()),
            };
            check
                .map_err(|e| StoreError::Invalid(format!("the edit would make {} unreadable: {e}", self.rel(guild))))?;
            edited.push((path, text));
        }
        for (path, text) in edited {
            write_atomic(&path, text.as_bytes(), 0o644)?;
        }
        Ok(())
    }
}

#[async_trait]
impl SettingsFiles for FsSettingsFiles {
    async fn load(&self) -> Result<(SettingsTree, Vec<FileError>), StoreError> {
        let me = self.clone();
        blocking(move || me.load_sync()).await
    }

    async fn write(&self, changes: &[Change]) -> Result<(), StoreError> {
        let me = self.clone();
        let changes = changes.to_vec();
        blocking(move || me.write_sync(&changes)).await
    }
}

// ------------------------------------------------------------------------------------------------ secrets

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SecretsToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    bot_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cookie_key: Option<String>,
    setup: SetupState,
}

/// `secrets.toml` (0600) and `setup-code` (0600, only while setup is unfinished).
#[derive(Debug, Clone)]
pub struct FsSecretsFile {
    path: PathBuf,
    code_path: PathBuf,
}

impl FsSecretsFile {
    pub fn new(data_dir: &Path) -> FsSecretsFile {
        FsSecretsFile {
            path: data_dir.join("secrets.toml"),
            code_path: data_dir.join("setup-code"),
        }
    }
}

fn secret(s: Option<String>) -> Option<SecretString> {
    s.filter(|s| !s.is_empty()).map(SecretString::from)
}

fn exposed(s: Option<&SecretString>) -> Option<String> {
    s.map(|s| s.expose_secret().to_owned())
}

#[async_trait]
impl SecretsFile for FsSecretsFile {
    async fn load(&self) -> Result<Secrets, StoreError> {
        let path = self.path.clone();
        blocking(move || {
            let Some(text) = read_optional(&path)? else {
                return Ok(Secrets::default());
            };
            let t: SecretsToml =
                toml::from_str(&text).map_err(|e| StoreError::Invalid(format!("secrets.toml: {}", e.message())))?;
            Ok(Secrets {
                bot_token: secret(t.bot_token),
                client_secret: secret(t.client_secret),
                cookie_key: secret(t.cookie_key),
                setup: t.setup,
            })
        })
        .await
    }

    async fn save(&self, s: &Secrets) -> Result<(), StoreError> {
        let t = SecretsToml {
            bot_token: exposed(s.bot_token.as_ref()),
            client_secret: exposed(s.client_secret.as_ref()),
            cookie_key: exposed(s.cookie_key.as_ref()),
            setup: s.setup.clone(),
        };
        let text = format!(
            "# Secrets of the profanity watch bot (set in the web UI). Keep this file private.\n{}",
            toml::to_string(&t).map_err(|e| StoreError::Invalid(e.to_string()))?
        );
        let path = self.path.clone();
        blocking(move || Ok(write_atomic(&path, text.as_bytes(), 0o600)?)).await
    }

    async fn write_setup_code(&self, code: Option<&str>) -> Result<(), StoreError> {
        let path = self.code_path.clone();
        let code = code.map(str::to_owned);
        blocking(move || {
            match code {
                Some(c) => write_atomic(&path, format!("{c}\n").as_bytes(), 0o600)?,
                None => match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(e.into()),
                },
            }
            Ok(())
        })
        .await
    }
}

// ------------------------------------------------------------------------------------------------ sessions

/// `sessions.json` (0600).
#[derive(Debug, Clone)]
pub struct FsSessionsFile {
    path: PathBuf,
}

impl FsSessionsFile {
    pub fn new(data_dir: &Path) -> FsSessionsFile {
        FsSessionsFile {
            path: data_dir.join("sessions.json"),
        }
    }
}

#[async_trait]
impl SessionsFile for FsSessionsFile {
    async fn load(&self) -> Result<BTreeMap<String, SessionRecord>, StoreError> {
        let path = self.path.clone();
        blocking(move || {
            let Some(text) = read_optional(&path)? else {
                return Ok(BTreeMap::new());
            };
            serde_json::from_str(&text).map_err(|e| StoreError::Invalid(format!("sessions.json: {e}")))
        })
        .await
    }

    async fn save(&self, sessions: &BTreeMap<String, SessionRecord>) -> Result<(), StoreError> {
        let text = serde_json::to_vec_pretty(sessions).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let path = self.path.clone();
        blocking(move || Ok(write_atomic(&path, &text, 0o600)?)).await
    }
}

/// `api.json` in the data directory.
#[derive(Debug)]
pub struct FsApiFile {
    path: PathBuf,
}

impl FsApiFile {
    pub fn new(data_dir: &Path) -> FsApiFile {
        FsApiFile {
            path: data_dir.join("api.json"),
        }
    }
}

#[async_trait]
impl ApiFile for FsApiFile {
    async fn load(&self) -> Result<ApiConfig, StoreError> {
        let path = self.path.clone();
        blocking(move || {
            let Some(text) = read_optional(&path)? else {
                return Ok(ApiConfig::default());
            };
            serde_json::from_str(&text).map_err(|e| StoreError::Invalid(format!("api.json: {e}")))
        })
        .await
    }

    async fn save(&self, config: &ApiConfig) -> Result<(), StoreError> {
        let text = serde_json::to_vec_pretty(config).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let path = self.path.clone();
        blocking(move || Ok(write_atomic(&path, &text, 0o600)?)).await
    }
}
