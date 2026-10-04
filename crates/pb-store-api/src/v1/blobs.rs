//! Content-addressed blobs: uploads, prepared renders, recordings. Immutable; deleted only on request.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;
use pb_domain::BlobHash;
use serde::{Deserialize, Serialize};

use super::log::StoreError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobInfo {
    pub hash: BlobHash,
    pub size: u64,
    /// Not there before (a new blob, not a duplicate).
    pub new: bool,
}

#[async_trait]
pub trait BlobStore: Send + Sync + 'static {
    /// Stores bytes (synced before `Ok`).
    async fn put(&self, bytes: Bytes) -> Result<BlobInfo, StoreError>;

    /// Moves a staged file in (hashing it on the way); the staged file is gone afterwards.
    async fn put_file(&self, staged: &Path) -> Result<BlobInfo, StoreError>;

    async fn get(&self, hash: &BlobHash) -> Result<Option<Bytes>, StoreError>;

    /// The file to stream from (`None` when not there).
    async fn path(&self, hash: &BlobHash) -> Option<PathBuf>;

    /// Removes the content; `false` when it was not there.
    async fn delete(&self, hash: &BlobHash) -> Result<bool, StoreError>;

    /// A place for staging uploads (emptied at start).
    fn staging_dir(&self) -> &Path;

    /// Bytes on disk.
    fn size(&self) -> u64;
}
