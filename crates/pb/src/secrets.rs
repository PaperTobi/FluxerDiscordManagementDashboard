//! `secrets.toml`, with the bot token and client secret from the environment winning when they are given there
//! (`PB_BOT_TOKEN`, `PB_CLIENT_SECRET`, or `PB_BOT_TOKEN_FILE` / `PB_CLIENT_SECRET_FILE` naming a file, which is how
//! podman secrets are mounted). Values from the environment are never written to the file.

use async_trait::async_trait;
use pb_store::FsSecretsFile;
use pb_store_api::{Secrets, SecretsFile, StoreError};
use secrecy::SecretString;

/// A secret from `NAME` or the file named by `NAME_FILE`.
fn from_env(name: &str) -> Result<Option<SecretString>, String> {
    if let Ok(v) = std::env::var(name) {
        let v = v.trim().to_owned();
        return Ok((!v.is_empty()).then(|| SecretString::from(v)));
    }
    let file_var = format!("{name}_FILE");
    match std::env::var(&file_var) {
        Ok(path) => match std::fs::read_to_string(&path) {
            Ok(v) => Ok(Some(SecretString::from(v.trim().to_owned()))),
            Err(e) => Err(format!("{file_var}={path}: {e}")),
        },
        Err(_) => Ok(None),
    }
}

pub struct EnvSecrets {
    file: FsSecretsFile,
    token: Option<SecretString>,
    client_secret: Option<SecretString>,
}

impl std::fmt::Debug for EnvSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvSecrets")
            .field("token_from_env", &self.token.is_some())
            .field("client_secret_from_env", &self.client_secret.is_some())
            .finish_non_exhaustive()
    }
}

impl EnvSecrets {
    pub fn new(file: FsSecretsFile) -> Result<EnvSecrets, String> {
        Ok(EnvSecrets::with(
            file,
            from_env("PB_BOT_TOKEN")?,
            from_env("PB_CLIENT_SECRET")?,
        ))
    }

    pub fn with(file: FsSecretsFile, token: Option<SecretString>, client_secret: Option<SecretString>) -> EnvSecrets {
        EnvSecrets {
            file,
            token,
            client_secret,
        }
    }

    /// Which secrets come from the environment (token, client secret).
    pub fn given_by_env(&self) -> (bool, bool) {
        (self.token.is_some(), self.client_secret.is_some())
    }
}

#[async_trait]
impl SecretsFile for EnvSecrets {
    async fn load(&self) -> Result<Secrets, StoreError> {
        let mut s = self.file.load().await?;
        if let Some(t) = &self.token {
            s.bot_token = Some(t.clone());
        }
        if let Some(c) = &self.client_secret {
            s.client_secret = Some(c.clone());
        }
        Ok(s)
    }

    async fn save(&self, s: &Secrets) -> Result<(), StoreError> {
        // What the environment gives stays out of the file (the file keeps its own value, if any).
        let on_disk = self.file.load().await?;
        let mut out = s.clone();
        if self.token.is_some() {
            out.bot_token = on_disk.bot_token;
        }
        if self.client_secret.is_some() {
            out.client_secret = on_disk.client_secret;
        }
        self.file.save(&out).await
    }

    async fn write_setup_code(&self, code: Option<&str>) -> Result<(), StoreError> {
        self.file.write_setup_code(code).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn token(s: &Secrets) -> Option<String> {
        s.bot_token.as_ref().map(|t| t.expose_secret().to_owned())
    }

    #[tokio::test]
    async fn the_environment_wins_and_is_never_written() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let file = FsSecretsFile::new(dir.path());
        let on_disk = Secrets {
            bot_token: Some(SecretString::from("1.file".to_owned())),
            ..Secrets::default()
        };
        file.save(&on_disk).await.unwrap_or_else(|e| panic!("{e}"));

        let env = EnvSecrets::with(
            FsSecretsFile::new(dir.path()),
            Some(SecretString::from("2.env".to_owned())),
            None,
        );
        let mut s = env.load().await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(token(&s).as_deref(), Some("2.env"));
        // Saving (the web UI after a login) keeps the file's own token; the client secret is written.
        s.bot_token = Some(SecretString::from("3.new".to_owned()));
        s.client_secret = Some(SecretString::from("cs".to_owned()));
        env.save(&s).await.unwrap_or_else(|e| panic!("{e}"));
        let raw = std::fs::read_to_string(dir.path().join("secrets.toml")).unwrap_or_default();
        assert!(
            raw.contains("1.file") && !raw.contains("2.env") && !raw.contains("3.new") && raw.contains("cs"),
            "{raw}"
        );
    }
}
