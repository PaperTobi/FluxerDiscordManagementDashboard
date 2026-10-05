//! The process configuration: `config.toml` in the data directory (optional), then environment variables
//! `PB__<SECTION>__<KEY>` (for example `PB__WEB__BIND=0.0.0.0:8790`). Settings people change live in the settings
//! files; `[defaults]` here only sets their starting values below the global settings.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub web: Web,
    pub inference: Inference,
    pub logging: Logging,
    /// Starting values for settings (below `settings/global.toml`).
    pub defaults: pb_settings::Layer,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Web {
    pub bind: SocketAddr,
    /// The built browser bundle (`pkg/` inside).
    pub site: PathBuf,
    /// Serve HTTPS with this certificate (browsers record from the microphone only over HTTPS or on localhost).
    pub tls: Option<Tls>,
    /// A browser on this machine that opens the web UI at localhost is the bot's owner, without logging in with
    /// Fluxer. The image turns it off (port forwarding there can make other machines look like this one).
    pub local_owner: bool,
}

impl Default for Web {
    fn default() -> Self {
        Web {
            bind: SocketAddr::from(([0, 0, 0, 0], 8790)),
            site: crate::layout::here().site.clone(),
            tls: None,
            local_owner: true,
        }
    }
}

/// PEM files: the certificate (then its chain) and its private key.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tls {
    pub cert: PathBuf,
    pub key: PathBuf,
}

impl Tls {
    fn read(path: &std::path::Path) -> Result<Vec<u8>, String> {
        std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The server configuration from the files.
    pub fn server_config(&self) -> Result<std::sync::Arc<rustls::ServerConfig>, String> {
        pb_tls::server_config(&Self::read(&self.cert)?, &Self::read(&self.key)?)
            .map_err(|e| format!("{} / {}: {e}", self.cert.display(), self.key.display()))
    }

    /// A client that trusts exactly this certificate (`pb health`).
    pub fn pinned_client(&self) -> Result<std::sync::Arc<rustls::ClientConfig>, String> {
        pb_tls::pinned_client_config(&Self::read(&self.cert)?).map_err(|e| format!("{}: {e}", self.cert.display()))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Device {
    /// The GPU when one can be used, else the CPU (the start log says which).
    #[default]
    Auto,
    Cpu,
    /// The first discrete GPU (Vulkan through wgpu); the bot does not start without it.
    Gpu,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Inference {
    /// The model weights (`silero-vad/`, `roblox-voice-safety-v3/`, `voices/`), as `pb fetch-weights` writes them.
    pub weights: PathBuf,
    /// The directory that holds espeak-ng's `espeak-ng-data` (the phonemizer the Piper voices were trained with).
    pub espeak_data: PathBuf,
    /// The voice clips shipped with the bot (WAV), used when no voice speaks a person's languages.
    pub clips: PathBuf,
    /// Where the classifier runs.
    pub device: Device,
}

impl Default for Inference {
    fn default() -> Self {
        Inference {
            weights: crate::layout::here().weights.clone(),
            espeak_data: crate::layout::here().espeak_data.clone(),
            clips: crate::layout::here().clips.clone(),
            device: Device::Auto,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Logging {
    /// A tracing filter (`info`, `warn,pb_engine=debug` …); `RUST_LOG` wins when set.
    pub level: String,
    /// Daily log files go here (default: `<data>/logs`). They are kept: delete old ones by hand if needed.
    pub dir: Option<PathBuf>,
}

impl Default for Logging {
    fn default() -> Self {
        Logging {
            level: "info".to_owned(),
            dir: None,
        }
    }
}

/// Reads the file (when it exists) and the environment.
pub fn load(file: &Path) -> Result<Config, String> {
    let c = config::Config::builder()
        .add_source(
            config::File::from(file)
                .format(config::FileFormat::Toml)
                .required(false),
        )
        .add_source(
            config::Environment::with_prefix("PB")
                .prefix_separator("__")
                .separator("__")
                .try_parsing(true),
        )
        .build()
        .map_err(|e| format!("{}: {e}", file.display()))?;
    c.try_deserialize().map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_then_environment() {
        let dir = std::env::temp_dir().join(format!("pb-config-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap_or_default();
        let f = dir.join("config.toml");
        std::fs::write(&f, "[web]\nbind = \"127.0.0.1:9000\"\n[inference]\ndevice = \"gpu\"\n").unwrap_or_default();
        let c = load(&f);
        assert!(c.is_ok(), "{c:?}");
        let c = c.unwrap_or_default();
        assert_eq!(c.web.bind, SocketAddr::from(([127, 0, 0, 1], 9000)));
        assert_eq!(c.inference.device, Device::Gpu);
        assert_eq!(c.logging.level, "info");
        let bad = dir.join("bad.toml");
        std::fs::write(&bad, "[web]\nport = 1\n").unwrap_or_default();
        assert!(load(&bad).is_err(), "unknown keys are rejected");
        let missing = load(&dir.join("none.toml"));
        assert!(missing.is_ok(), "a missing file is fine");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
