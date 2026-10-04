//! The files beside the log: settings (TOML, hand-editable), secrets and login sessions.

use std::collections::BTreeMap;

use async_trait::async_trait;
use jiff::Timestamp;
use pb_domain::UserId;
use pb_settings::{Change, FileError, SettingsTree};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};

use super::log::StoreError;

/// The settings files: `settings/global.toml` and `settings/servers/<community>.toml`.
#[async_trait]
pub trait SettingsFiles: Send + Sync + 'static {
    /// Reads every file. Files that fail are reported and left out (at start that is fatal; at runtime the old
    /// values stay in use).
    async fn load(&self) -> Result<(SettingsTree, Vec<FileError>), StoreError>;

    /// Edits the files the changes touch in place (comments and layout kept), each replaced atomically. Every edit is
    /// checked before any file is written: a refused one leaves all files as they were.
    async fn write(&self, changes: &[Change]) -> Result<(), StoreError>;
}

/// Setup progress.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SetupState {
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<UserId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<Timestamp>,
}

/// `secrets.toml` (mode 0600).
#[derive(Debug, Clone, Default)]
pub struct Secrets {
    pub bot_token: Option<SecretString>,
    pub client_secret: Option<SecretString>,
    /// HMAC key for session cookies (created at first start).
    pub cookie_key: Option<SecretString>,
    pub setup: SetupState,
}

#[async_trait]
pub trait SecretsFile: Send + Sync + 'static {
    async fn load(&self) -> Result<Secrets, StoreError>;
    /// Replaces the file atomically (mode 0600).
    async fn save(&self, secrets: &Secrets) -> Result<(), StoreError>;
    /// The current setup code (written while setup is unfinished, so the operator can read it).
    async fn write_setup_code(&self, code: Option<&str>) -> Result<(), StoreError>;
}

/// A login session (keyed by the SHA-256 of its id; the id itself is only in the cookie).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub user: UserId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    pub owner: bool,
    pub created: Timestamp,
    pub expires: Timestamp,
    /// Sensitive actions need a login newer than this.
    pub fresh_until: Timestamp,
    pub last_seen: Timestamp,
}

#[async_trait]
pub trait SessionsFile: Send + Sync + 'static {
    async fn load(&self) -> Result<BTreeMap<String, SessionRecord>, StoreError>;
    /// Replaces the file atomically (mode 0600).
    async fn save(&self, sessions: &BTreeMap<String, SessionRecord>) -> Result<(), StoreError>;
}
