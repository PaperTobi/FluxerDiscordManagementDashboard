//! Small file-system helpers: atomic replace, directory sync, the data-directory lock.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Writes `bytes` to `path` atomically: a temp file beside it, synced, renamed over, the directory synced.
/// `mode` applies to the new file (unix).
pub fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    create_dir_all_synced(dir)?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.{}.tmp", unique()));
    {
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(mode);
        let mut f = opts.open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))?;
    }
    fs::rename(&tmp, path)?;
    sync_dir(dir)
}

/// Makes a rename or a new file in `dir` durable.
pub fn sync_dir(dir: &Path) -> std::io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Creates `dir` and any missing parents, each made durable in its own parent.
pub fn create_dir_all_synced(dir: &Path) -> std::io::Result<()> {
    let mut missing = Vec::new();
    let mut at = dir;
    while !at.exists() {
        missing.push(at);
        match at.parent() {
            Some(p) if !p.as_os_str().is_empty() => at = p,
            _ => break,
        }
    }
    for d in missing.into_iter().rev() {
        match fs::create_dir(d) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
        if let Some(p) = d.parent().filter(|p| !p.as_os_str().is_empty()) {
            sync_dir(p)?;
        }
    }
    Ok(())
}

/// A name part no other temp file of this process uses (two writers of the same file never share one).
pub fn unique() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

/// The exclusive lock on the data directory, held for the life of the process.
#[derive(Debug)]
pub struct DataLock {
    _file: File,
    pub path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("another bot process uses {0} (its lock file is held)")]
    Held(PathBuf),
    #[error("cannot lock {0}: {1}")]
    Io(PathBuf, std::io::Error),
}

impl DataLock {
    /// Takes `<data>/.pb.lock`; fails at once if another process holds it.
    pub fn take(data_dir: &Path) -> Result<DataLock, LockError> {
        let path = data_dir.join(".pb.lock");
        fs::create_dir_all(data_dir).map_err(|e| LockError::Io(path.clone(), e))?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| LockError::Io(path.clone(), e))?;
        match file.try_lock() {
            Ok(()) => Ok(DataLock { _file: file, path }),
            Err(fs::TryLockError::WouldBlock) => Err(LockError::Held(path)),
            Err(fs::TryLockError::Error(e)) => Err(LockError::Io(path, e)),
        }
    }
}
