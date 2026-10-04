//! What the engine needs from outside, and its clock.

use std::sync::Arc;

use jiff::Timestamp;
use pb_domain::BlobHash;
use pb_fluxer_api::Fluxer;
use pb_infer::Inference;
use pb_live::Hub;
use pb_store_api::{BlobStore, EventLog, Index, SecretsFile, SettingsFiles};
use pb_voice_api::VoiceTransport;

/// Time: a monotonic clock in seconds (timers, the follow machine) and the wall clock (records).
pub trait Clock: Send + Sync + 'static {
    fn mono(&self) -> f64;
    fn now(&self) -> Timestamp;
}

/// The system clock. Monotonic time is tokio's, so tests with paused time move it.
#[derive(Debug)]
pub struct SystemClock {
    start: tokio::time::Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock {
            start: tokio::time::Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn mono(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// A shipped clip (fallback when no voice speaks any of a person's languages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShippedClip {
    pub hash: BlobHash,
    pub text: String,
}

/// Everything the engine runs on.
#[derive(Clone)]
pub struct Deps {
    pub fluxer: Arc<dyn Fluxer>,
    pub voice: Arc<dyn VoiceTransport>,
    pub inference: Inference,
    pub log: Arc<dyn EventLog>,
    pub index: Arc<dyn Index>,
    pub blobs: Arc<dyn BlobStore>,
    pub settings_files: Arc<dyn SettingsFiles>,
    pub secrets: Arc<dyn SecretsFile>,
    pub hub: Hub,
    pub clock: Arc<dyn Clock>,
    /// The bot's version (for the System page and `bot.started`).
    pub version: String,
    /// Shipped clips, already in the blob store.
    pub shipped_clips: Vec<ShippedClip>,
    /// Free bytes on the data volume (for the System page; `None` when unknown).
    pub disk_free: Arc<dyn Fn() -> Option<u64> + Send + Sync>,
}

impl std::fmt::Debug for Deps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Deps")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}
