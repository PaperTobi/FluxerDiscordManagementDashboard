//! The event log.

use std::sync::Arc;

use async_trait::async_trait;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::envelope::{LineHash, NewEvent, StoredEvent};

/// Where an event landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRef {
    pub seq: u64,
    pub hash: LineHash,
}

/// The writer's condition.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WriterHealth {
    #[default]
    Ok,
    /// A write or sync failed (a full disk included). The file was cut back to the last good line and nothing is
    /// written until [`EventLog::retry`] (an owner's request) or the next start. Moderation goes on in memory.
    Halted { error: String, since_seq: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("the event log stopped writing after an error: {0}")]
    Halted(String),
    #[error("i/o: {0}")]
    Io(String),
    #[error("the log is damaged: {0}")]
    Corrupt(String),
    #[error("the index: {0}")]
    Index(String),
    #[error("invalid: {0}")]
    Invalid(String),
}

impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e.to_string())
    }
}

/// A problem `verify` found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyProblem {
    pub segment: String,
    pub line: u64,
    pub message: String,
}

/// The result of checking every line and the hash chain.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VerifyReport {
    pub segments: u64,
    pub events: u64,
    pub bytes: u64,
    pub problems: Vec<VerifyProblem>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// The add-only, hash-chained event log.
#[async_trait]
pub trait EventLog: Send + Sync + 'static {
    /// Appends events in order (group commit): returns once they are synced to disk. No event is ever lost
    /// silently: on failure nothing of the batch is in the log and the error says why.
    async fn append(&self, events: Vec<NewEvent>) -> Result<Vec<EventRef>, StoreError>;

    /// The last event.
    fn head(&self) -> Option<EventRef>;

    fn health(&self) -> WriterHealth;

    /// After a halt: try writing again (an owner's request).
    async fn retry(&self) -> Result<(), StoreError>;

    /// Every event from `from_seq` on (1 is the first), in order.
    fn scan(&self, from_seq: u64) -> BoxStream<'_, Result<StoredEvent, StoreError>>;

    /// Re-reads everything and checks each line and the hash chain.
    async fn verify(&self) -> Result<VerifyReport, StoreError>;

    /// Batches of events as they are committed. A receiver that lags gets `Lagged` and catches up with `scan`.
    fn follow(&self) -> broadcast::Receiver<Arc<[StoredEvent]>>;

    /// Bytes on disk.
    fn size(&self) -> u64;
}
