use anyhow::{Context, Result};
use nix::fcntl::{Flock, FlockArg};
use std::fs::{File, OpenOptions};
use std::path::Path;
use tracing::debug;

/// RAII guard that holds an exclusive advisory lock on a `.muxix.lock` file in
/// the given directory. Serializes concurrent muxix processes that rewrite a
/// shared file there: `.git/config`, the state dir's tombstone set.
pub struct DirLock {
    _lock: Flock<File>,
}

impl DirLock {
    /// Acquire an exclusive lock, blocking until available.
    pub fn acquire(git_common_dir: &Path) -> Result<Self> {
        let lock_path = git_common_dir.join(".muxix.lock");
        debug!(path = %lock_path.display(), "config_lock:acquiring");

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .with_context(|| format!("Failed to open lock file: {}", lock_path.display()))?;

        let lock = Flock::lock(file, FlockArg::LockExclusive)
            .map_err(|(_file, errno)| errno)
            .with_context(|| format!("Failed to acquire lock: {}", lock_path.display()))?;

        debug!(path = %lock_path.display(), "config_lock:acquired");
        Ok(Self { _lock: lock })
    }
}
