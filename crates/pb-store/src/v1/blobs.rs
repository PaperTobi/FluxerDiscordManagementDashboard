//! Content-addressed blobs at `blobs/sha256/aa/bb/<hex>`, staged uploads in `tmp/`.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use async_trait::async_trait;
use bytes::Bytes;
use pb_domain::BlobHash;
use pb_store_api::{BlobInfo, BlobStore, StoreError};
use sha2::{Digest, Sha256};

use super::fsutil::{create_dir_all_synced, sync_dir, unique};

/// Blobs on the file system.
#[derive(Debug, Clone)]
pub struct FsBlobStore {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    root: PathBuf,
    staging: PathBuf,
    bytes: AtomicU64,
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(_) => e.metadata().map_or(0, |m| m.len()),
            Err(_) => 0,
        })
        .sum()
}

impl FsBlobStore {
    /// Opens `blobs_dir` and empties `staging_dir` (left-overs of interrupted uploads).
    pub async fn open(blobs_dir: &Path, staging_dir: &Path) -> Result<FsBlobStore, StoreError> {
        let (blobs_dir, staging_dir) = (blobs_dir.to_owned(), staging_dir.to_owned());
        tokio::task::spawn_blocking(move || FsBlobStore::open_sync(&blobs_dir, &staging_dir))
            .await
            .map_err(|e| StoreError::Io(e.to_string()))?
    }

    fn open_sync(blobs_dir: &Path, staging_dir: &Path) -> Result<FsBlobStore, StoreError> {
        let root = blobs_dir.join("sha256");
        fs::create_dir_all(&root)?;
        if staging_dir.exists() {
            for e in fs::read_dir(staging_dir)?.flatten() {
                let p = e.path();
                let _ = if p.is_dir() {
                    fs::remove_dir_all(&p)
                } else {
                    fs::remove_file(&p)
                };
            }
        }
        fs::create_dir_all(staging_dir)?;
        let bytes = AtomicU64::new(dir_size(&root));
        Ok(FsBlobStore {
            inner: Arc::new(Inner {
                root,
                staging: staging_dir.to_owned(),
                bytes,
            }),
        })
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Inner) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, StoreError> {
        let inner = self.inner.clone();
        tokio::task::spawn_blocking(move || f(&inner))
            .await
            .map_err(|e| StoreError::Io(e.to_string()))?
    }
}

impl Inner {
    fn location(&self, hash: &BlobHash) -> PathBuf {
        let hex = hash.hex();
        self.root.join(&hex[0..2]).join(&hex[2..4]).join(hex)
    }

    /// Moves a synced temp file into place (or drops it when the content is already there).
    fn install(&self, tmp: &Path, hash: BlobHash, size: u64) -> Result<BlobInfo, StoreError> {
        let dest = self.location(&hash);
        if dest.exists() {
            fs::remove_file(tmp)?;
            return Ok(BlobInfo { hash, size, new: false });
        }
        let dir = dest.parent().unwrap_or(&self.root);
        create_dir_all_synced(dir)?;
        fs::rename(tmp, &dest)?;
        sync_dir(dir)?;
        self.bytes.fetch_add(size, Ordering::Relaxed);
        Ok(BlobInfo { hash, size, new: true })
    }

    fn put_sync(&self, bytes: &[u8]) -> Result<BlobInfo, StoreError> {
        let hash = BlobHash::from_bytes(Sha256::digest(bytes).into());
        if self.location(&hash).exists() {
            return Ok(BlobInfo {
                hash,
                size: bytes.len() as u64,
                new: false,
            });
        }
        let tmp = self.staging.join(format!(".put-{}-{}", hash.hex(), unique()));
        {
            let mut f = File::create(&tmp)?;
            f.write_all(bytes)?;
            f.sync_all()?;
        }
        self.install(&tmp, hash, bytes.len() as u64)
    }

    fn put_file_sync(&self, staged: &Path) -> Result<BlobInfo, StoreError> {
        let mut f = File::open(staged)?;
        let mut hasher = Sha256::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut size = 0u64;
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            size += n as u64;
        }
        f.sync_all()?;
        drop(f);
        self.install(staged, BlobHash::from_bytes(hasher.finalize().into()), size)
    }
}

#[async_trait]
impl BlobStore for FsBlobStore {
    async fn put(&self, bytes: Bytes) -> Result<BlobInfo, StoreError> {
        self.blocking(move |i| i.put_sync(&bytes)).await
    }

    async fn put_file(&self, staged: &Path) -> Result<BlobInfo, StoreError> {
        let staged = staged.to_owned();
        self.blocking(move |i| i.put_file_sync(&staged)).await
    }

    async fn get(&self, hash: &BlobHash) -> Result<Option<Bytes>, StoreError> {
        match tokio::fs::read(self.inner.location(hash)).await {
            Ok(b) => Ok(Some(b.into())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn path(&self, hash: &BlobHash) -> Option<PathBuf> {
        let p = self.inner.location(hash);
        tokio::fs::try_exists(&p).await.ok()?.then_some(p)
    }

    async fn delete(&self, hash: &BlobHash) -> Result<bool, StoreError> {
        let p = self.inner.location(hash);
        let size = match tokio::fs::metadata(&p).await {
            Ok(m) => m.len(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(e) => return Err(e.into()),
        };
        tokio::fs::remove_file(&p).await?;
        if let Some(dir) = p.parent() {
            let dir = dir.to_owned();
            self.blocking(move |_| Ok(sync_dir(&dir)?)).await?;
        }
        let bytes = &self.inner.bytes;
        bytes.fetch_sub(size.min(bytes.load(Ordering::Relaxed)), Ordering::Relaxed);
        Ok(true)
    }

    fn staging_dir(&self) -> &Path {
        &self.inner.staging
    }

    fn size(&self) -> u64 {
        self.inner.bytes.load(Ordering::Relaxed)
    }
}
