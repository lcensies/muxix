//! A cross-process advisory file lock used to serialize read-modify-write of
//! `project.json`.
//!
//! Acquisition is via `OpenOptions::create_new` (which maps to `O_CREAT |
//! O_EXCL` on Unix) so exactly one of two racing actors wins the create. This
//! is what makes the capability compare-and-swap (`Absent` → `InProgress`)
//! atomic: the loser blocks until the winner releases, then re-reads the state.
//!
//! A holder that crashes mid-section would otherwise deadlock every later
//! actor, so the lock is reclaimable: when the existing lockfile is older than
//! `lock_ttl`, a waiter steals it via an atomic `rename` (only one racer's
//! rename can succeed once the source name is gone).

use anyhow::{Context, Result, bail};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, warn};

/// How long to keep retrying before giving up acquiring the lock.
const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
/// Delay between acquisition attempts.
const POLL_INTERVAL: Duration = Duration::from_millis(25);
/// Age after which a held lockfile is presumed abandoned and may be stolen.
const DEFAULT_LOCK_TTL: Duration = Duration::from_secs(30);

/// RAII guard for an acquired lockfile. Releases (removes the file) on drop.
#[derive(Debug)]
pub struct FileLock {
    path: PathBuf,
    /// Set to false when this guard no longer owns the file (it was stolen).
    held: bool,
}

impl FileLock {
    /// Acquire the lock at `path`, blocking up to the default timeout and
    /// reclaiming the file if a previous holder left it stale.
    pub fn acquire(path: &Path) -> Result<Self> {
        Self::acquire_with(path, DEFAULT_ACQUIRE_TIMEOUT, DEFAULT_LOCK_TTL)
    }

    /// Acquire with explicit timeout/TTL (used by tests for fast, deterministic
    /// staleness checks).
    pub fn acquire_with(path: &Path, timeout: Duration, lock_ttl: Duration) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create lock directory")?;
        }

        let deadline = Instant::now() + timeout;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(path) {
                Ok(mut file) => {
                    // Best-effort owner marker for debugging / manual recovery.
                    let _ = writeln!(file, "{} {}", std::process::id(), now_secs());
                    return Ok(FileLock {
                        path: path.to_path_buf(),
                        held: true,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if lock_is_stale(path, lock_ttl) {
                        // Steal atomically: whoever wins the rename owns the
                        // removal; everyone else falls through and retries the
                        // create, where O_EXCL again picks a single winner.
                        steal_stale_lock(path);
                        continue;
                    }
                    if Instant::now() >= deadline {
                        bail!(
                            "timed out acquiring project state lock at {} (held by another actor)",
                            path.display()
                        );
                    }
                    std::thread::sleep(POLL_INTERVAL);
                }
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("Failed to acquire lock at {}", path.display()));
                }
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        if self.held {
            if let Err(e) = fs::remove_file(&self.path) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    warn!(path = %self.path.display(), error = %e, "failed to release project state lock");
                }
            }
        }
    }
}

/// Whether the lockfile is older than `ttl` (presumed abandoned by a crash).
fn lock_is_stale(path: &Path, ttl: Duration) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        // Gone already — not stale, just race; let the caller retry the create.
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    match SystemTime::now().duration_since(modified) {
        Ok(age) => age > ttl,
        // mtime in the future (clock skew): treat as fresh, don't steal.
        Err(_) => false,
    }
}

/// Attempt to remove a stale lockfile via an atomic rename so only one of
/// several racing waiters actually deletes it. Any failure is benign: it means
/// another waiter already stole it, and the caller will simply retry.
fn steal_stale_lock(path: &Path) {
    let steal_path = path.with_extension(format!("steal.{}", std::process::id()));
    match fs::rename(path, &steal_path) {
        Ok(()) => {
            debug!(path = %path.display(), "reclaimed stale project state lock");
            let _ = fs::remove_file(&steal_path);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Another waiter won the steal; nothing to do.
        }
        Err(e) => {
            warn!(path = %path.display(), error = %e, "failed to steal stale lock");
        }
    }
}

/// Current unix time in seconds (saturating at 0 before the epoch).
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn acquire_creates_and_release_removes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project.lock");

        {
            let _guard = FileLock::acquire(&path).unwrap();
            assert!(path.exists(), "lockfile exists while held");
        }
        assert!(!path.exists(), "lockfile removed on drop");
    }

    #[test]
    fn second_acquire_times_out_while_held() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project.lock");

        let _guard = FileLock::acquire(&path).unwrap();
        // A fresh lock with a long TTL must not be stolen; a short timeout fails.
        let err =
            FileLock::acquire_with(&path, Duration::from_millis(80), Duration::from_secs(3600))
                .unwrap_err();
        assert!(err.to_string().contains("timed out"), "got: {err}");
    }

    #[test]
    fn stale_lock_is_stolen() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project.lock");

        // Simulate a crashed holder: a lockfile whose mtime is well in the past.
        fs::write(&path, "99999 0").unwrap();
        let old = filetime::FileTime::from_unix_time(1, 0);
        filetime::set_file_mtime(&path, old).unwrap();

        // With a 1s TTL the lock is stale and must be reclaimed promptly.
        let guard =
            FileLock::acquire_with(&path, Duration::from_millis(500), Duration::from_secs(1))
                .unwrap();
        assert!(path.exists());
        drop(guard);
        assert!(!path.exists());
    }
}
