//! Checkpoint and restore for sandbox agents.
//!
//! Supports microsandbox (microVM snapshots) and Docker/Podman (CRIU-based
//! `docker checkpoint`). Strategy is configurable per-project.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

use crate::config::{CheckpointConfig, SandboxBackend, SandboxConfig};
use crate::state::{AgentState, StateStore};

/// Checkpoint a running agent sandbox and record the snapshot path in state.
///
/// - MicroSandbox backend: calls `msb sandbox snapshot`
/// - Container backend: calls `docker checkpoint create` (requires CRIU)
/// - Lima backend: not supported (returns Ok without doing anything)
pub fn checkpoint_agent(
    state: &AgentState,
    sandbox_config: &SandboxConfig,
    store: &StateStore,
) -> Result<PathBuf> {
    let sandbox_id = match &state.sandbox_id {
        Some(id) => id.clone(),
        None => bail!(
            "agent pane {} has no sandbox_id — cannot checkpoint",
            state.pane_key.pane_id
        ),
    };

    let checkpoint_dir = resolve_checkpoint_dir(&sandbox_config.checkpoint)?;
    let ts = unix_now();
    // Use millisecond resolution in the filename so two checkpoints within the
    // same second don't collide (state stores `checkpoint_ts` in seconds).
    let snapshot_name = format!("{}-{}.snap", sandbox_id, unix_now_millis());
    let snapshot_path = checkpoint_dir.join(&snapshot_name);

    match sandbox_config.backend() {
        SandboxBackend::MicroSandbox => {
            info!(sandbox_id, path = %snapshot_path.display(), "checkpointing microsandbox VM");
            super::microsandbox::snapshot(&sandbox_id, &snapshot_path)
                .context("microsandbox snapshot failed")?;
        }
        SandboxBackend::Container => {
            checkpoint_container(&sandbox_id, &snapshot_path, sandbox_config)?;
        }
        SandboxBackend::Lima => {
            warn!(
                sandbox_id,
                "checkpoint not supported for Lima backend — skipping"
            );
            return Ok(snapshot_path);
        }
    }

    // Update persisted state with checkpoint metadata
    match store
        .get_agent(&state.pane_key)
        .and_then(|o| o.ok_or_else(|| anyhow::anyhow!("agent state not found")))
    {
        Ok(mut fresh) => {
            fresh.checkpoint_path = Some(snapshot_path.clone());
            fresh.checkpoint_ts = Some(ts);
            if let Err(e) = store.upsert_agent(&fresh) {
                warn!(error = %e, "failed to update checkpoint_path in state");
            }
        }
        Err(e) => {
            // The on-disk snapshot exists but its path was never recorded, so
            // restore (which requires `checkpoint_path`) can't find it. Surface
            // this rather than silently leaving an orphaned, unresumable snapshot.
            warn!(
                error = %e,
                path = %snapshot_path.display(),
                "checkpoint written but agent state could not be updated; snapshot will not be resumable"
            );
        }
    }

    info!(path = %snapshot_path.display(), "checkpoint complete");

    // Prune old snapshots for this sandbox according to retention policy.
    prune_old_snapshots(
        &sandbox_id,
        &checkpoint_dir,
        &snapshot_path,
        &sandbox_config.checkpoint,
    );

    Ok(snapshot_path)
}

/// Restore a sandbox agent from its last checkpoint.
///
/// Returns an error if no checkpoint_path is recorded in state.
pub fn restore_agent(state: &AgentState, sandbox_config: &SandboxConfig) -> Result<()> {
    let sandbox_id = state
        .sandbox_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("agent has no sandbox_id"))?;

    let snapshot_path = state
        .checkpoint_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no checkpoint recorded for agent {}", sandbox_id))?;

    if !snapshot_path.exists() {
        bail!("checkpoint file not found: {}", snapshot_path.display());
    }

    info!(sandbox_id, path = %snapshot_path.display(), "restoring agent from checkpoint");

    match sandbox_config.backend() {
        SandboxBackend::MicroSandbox => {
            super::microsandbox::restore(sandbox_id, snapshot_path)
                .context("microsandbox restore failed")?;
        }
        SandboxBackend::Container => {
            restore_container(sandbox_id, snapshot_path, sandbox_config)?;
        }
        SandboxBackend::Lima => {
            bail!("restore not supported for Lima backend");
        }
    }

    info!(sandbox_id, "restore complete");
    Ok(())
}

/// Find the most recent snapshot file for `sandbox_id`, if any exists.
///
/// Used at launch time to decide whether to restore a sandbox (resume) or
/// create it fresh. Returns the newest `<sandbox_id>-*.snap` in the checkpoint
/// directory by filename (filenames embed a millisecond timestamp).
pub fn latest_snapshot(sandbox_id: &str, sandbox_config: &SandboxConfig) -> Option<PathBuf> {
    let dir = resolve_checkpoint_dir(&sandbox_config.checkpoint).ok()?;
    let prefix = format!("{}-", sandbox_id);
    let mut snaps: Vec<PathBuf> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with(&prefix) && n.ends_with(".snap"))
                .unwrap_or(false)
        })
        .collect();
    // Filenames embed a monotonic millisecond stamp, so lexical max = newest.
    snaps.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    snaps.pop()
}

// ── internals ────────────────────────────────────────────────────────────────

/// Delete old snapshots for `sandbox_id`, keeping only the `keep` most recent.
///
/// Snapshots for a sandbox are identified by the filename prefix `<sandbox_id>-`.
/// They are sorted by modification time (newest first); everything beyond the
/// keep limit is removed. The snapshot we just wrote (`current`) is always kept
/// regardless of ordering.
///
/// Failures are logged as warnings rather than hard errors — a full disk is bad
/// enough without also crashing the agent.
fn prune_old_snapshots(
    sandbox_id: &str,
    checkpoint_dir: &Path,
    current: &Path,
    config: &CheckpointConfig,
) {
    let keep = config.keep();
    if keep == 0 {
        return; // unlimited retention
    }

    let prefix = format!("{}-", sandbox_id);

    let mut snaps: Vec<PathBuf> = match std::fs::read_dir(checkpoint_dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with(&prefix) && n.ends_with(".snap"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(e) => {
            warn!(error = %e, "could not read checkpoint dir for pruning");
            return;
        }
    };

    // Sort newest-first: primary key = mtime, secondary = filename descending
    // (filenames encode the unix timestamp, so this is a reliable tiebreaker
    // when files are created faster than filesystem mtime resolution).
    snaps.sort_by(|a, b| {
        let mt = |p: &Path| p.metadata().and_then(|m| m.modified()).ok();
        mt(b)
            .cmp(&mt(a))
            .then_with(|| b.file_name().cmp(&a.file_name()))
    });

    // Ensure the snapshot we just wrote is at the front.
    if let Some(pos) = snaps.iter().position(|p| p == current) {
        snaps.swap(0, pos);
    }

    for stale in snaps.iter().skip(keep) {
        match std::fs::remove_file(stale) {
            Ok(()) => info!(path = %stale.display(), "pruned old checkpoint"),
            Err(e) => warn!(path = %stale.display(), error = %e, "failed to prune checkpoint"),
        }
    }
}

fn resolve_checkpoint_dir(config: &CheckpointConfig) -> Result<PathBuf> {
    let dir = if let Some(ref d) = config.dir {
        crate::util::expand_tilde(d)
    } else {
        crate::xdg::state_dir()?.join("checkpoints")
    };
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create checkpoint dir {}", dir.display()))?;
    Ok(dir)
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn unix_now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Container checkpoint/restore is CRIU-based. Both Docker (experimental
/// `docker checkpoint`) and Podman (`podman container checkpoint`) shell out to
/// the `criu` binary, so it must be installed on the host.
fn ensure_criu_available() -> Result<()> {
    if which::which("criu").is_err() {
        bail!(
            "container checkpoint/restore requires CRIU, but the `criu` binary was not found.\n\
             Install it (e.g. `apt install criu` / `dnf install criu`) — see \
             https://github.com/checkpoint-restore/criu — and ensure your container \
             runtime has checkpoint support enabled (Docker needs experimental mode; \
             rootless Podman cannot checkpoint — run Podman rootful, i.e. as root or \
             via a rootful Podman service)."
        );
    }
    Ok(())
}

/// Stable checkpoint name derived from the snapshot file (used as the CRIU
/// checkpoint label for the dir-based Docker path).
fn checkpoint_label(snapshot_path: &Path) -> String {
    snapshot_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("mx-checkpoint")
        .to_string()
}

// ── pure argv builders (unit-tested) ──────────────────────────────────────────

/// `podman container checkpoint --leave-running --export <file> <id>`
fn podman_checkpoint_argv(export_path: &str, container_id: &str) -> Vec<String> {
    vec![
        "container".into(),
        "checkpoint".into(),
        "--leave-running".into(),
        "--export".into(),
        export_path.into(),
        container_id.into(),
    ]
}

/// `podman container restore --import <file>`
/// (restore re-creates the container under its original name from the archive)
fn podman_restore_argv(import_path: &str) -> Vec<String> {
    vec![
        "container".into(),
        "restore".into(),
        "--import".into(),
        import_path.into(),
    ]
}

/// `docker checkpoint create --leave-running --checkpoint-dir <dir> <id> <name>`
fn docker_checkpoint_argv(checkpoint_dir: &str, container_id: &str, name: &str) -> Vec<String> {
    vec![
        "checkpoint".into(),
        "create".into(),
        "--leave-running".into(),
        "--checkpoint-dir".into(),
        checkpoint_dir.into(),
        container_id.into(),
        name.into(),
    ]
}

/// `docker start --checkpoint-dir <dir> --checkpoint <name> <id>`
fn docker_restore_argv(checkpoint_dir: &str, name: &str, container_id: &str) -> Vec<String> {
    vec![
        "start".into(),
        "--checkpoint-dir".into(),
        checkpoint_dir.into(),
        "--checkpoint".into(),
        name.into(),
        container_id.into(),
    ]
}

fn run_runtime(runtime: &str, argv: &[String], what: &str) -> Result<()> {
    let status = std::process::Command::new(runtime)
        .args(argv)
        .status()
        .with_context(|| format!("failed to run `{runtime} {}`", argv.join(" ")))?;
    if !status.success() {
        bail!("{what} failed (exit {:?})", status.code());
    }
    Ok(())
}

fn checkpoint_container(
    container_id: &str,
    snapshot_path: &Path,
    config: &SandboxConfig,
) -> Result<()> {
    use crate::config::SandboxRuntime;
    ensure_criu_available()?;

    let runtime = config.runtime();
    let bin = runtime.binary_name();
    let path_str = snapshot_path.to_string_lossy();
    info!(container_id, runtime = bin, path = %path_str, "checkpointing container (CRIU)");

    match runtime {
        SandboxRuntime::Podman => {
            // Podman exports a self-contained archive — our snapshot file directly.
            run_runtime(
                bin,
                &podman_checkpoint_argv(&path_str, container_id),
                "podman container checkpoint",
            )
        }
        SandboxRuntime::Docker => {
            // Docker writes a checkpoint *directory*; tar it into our single-file
            // snapshot so retention/pruning treats every backend uniformly.
            let name = checkpoint_label(snapshot_path);
            let tmp = tempfile::tempdir().context("create temp checkpoint dir")?;
            let tmp_dir = tmp.path().to_string_lossy();
            run_runtime(
                bin,
                &docker_checkpoint_argv(&tmp_dir, container_id, &name),
                "docker checkpoint create",
            )?;
            tar_dir(tmp.path(), &name, snapshot_path).context("archive docker checkpoint")?;
            Ok(())
        }
        SandboxRuntime::AppleContainer => {
            bail!("checkpoint/restore is not supported for the Apple Container runtime")
        }
    }
}

fn restore_container(
    container_id: &str,
    snapshot_path: &Path,
    config: &SandboxConfig,
) -> Result<()> {
    use crate::config::SandboxRuntime;
    ensure_criu_available()?;

    let runtime = config.runtime();
    let bin = runtime.binary_name();
    let path_str = snapshot_path.to_string_lossy();
    info!(container_id, runtime = bin, path = %path_str, "restoring container (CRIU)");

    match runtime {
        SandboxRuntime::Podman => run_runtime(
            bin,
            &podman_restore_argv(&path_str),
            "podman container restore",
        ),
        SandboxRuntime::Docker => {
            let name = checkpoint_label(snapshot_path);
            let tmp = tempfile::tempdir().context("create temp checkpoint dir")?;
            untar_into(snapshot_path, tmp.path()).context("extract docker checkpoint")?;
            let tmp_dir = tmp.path().to_string_lossy();
            run_runtime(
                bin,
                &docker_restore_argv(&tmp_dir, &name, container_id),
                "docker start --checkpoint",
            )
        }
        SandboxRuntime::AppleContainer => {
            bail!("checkpoint/restore is not supported for the Apple Container runtime")
        }
    }
}

/// `tar -C <parent> -cf <archive> <entry>` — package a checkpoint dir into one file.
fn tar_dir(parent: &Path, entry: &str, archive: &Path) -> Result<()> {
    let status = std::process::Command::new("tar")
        .arg("-C")
        .arg(parent)
        .arg("-cf")
        .arg(archive)
        .arg(entry)
        .status()
        .context("run tar to package checkpoint")?;
    if !status.success() {
        bail!("tar failed packaging checkpoint into {}", archive.display());
    }
    Ok(())
}

/// `tar -C <dest> -xf <archive>` — unpack a checkpoint archive.
fn untar_into(archive: &Path, dest: &Path) -> Result<()> {
    let status = std::process::Command::new("tar")
        .arg("-C")
        .arg(dest)
        .arg("-xf")
        .arg(archive)
        .status()
        .context("run tar to extract checkpoint")?;
    if !status.success() {
        bail!("tar failed extracting checkpoint {}", archive.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CheckpointStrategy;

    #[test]
    fn podman_checkpoint_argv_exports_to_file() {
        let argv = podman_checkpoint_argv("/snaps/mx-x-1.snap", "mx-x");
        assert_eq!(
            argv,
            vec![
                "container",
                "checkpoint",
                "--leave-running",
                "--export",
                "/snaps/mx-x-1.snap",
                "mx-x",
            ]
        );
    }

    #[test]
    fn podman_restore_argv_imports_file() {
        assert_eq!(
            podman_restore_argv("/snaps/mx-x-1.snap"),
            vec!["container", "restore", "--import", "/snaps/mx-x-1.snap"]
        );
    }

    #[test]
    fn docker_checkpoint_and_restore_argv_use_checkpoint_dir() {
        assert_eq!(
            docker_checkpoint_argv("/tmp/cp", "mx-x", "mx-x-1"),
            vec![
                "checkpoint",
                "create",
                "--leave-running",
                "--checkpoint-dir",
                "/tmp/cp",
                "mx-x",
                "mx-x-1",
            ]
        );
        assert_eq!(
            docker_restore_argv("/tmp/cp", "mx-x-1", "mx-x"),
            vec![
                "start",
                "--checkpoint-dir",
                "/tmp/cp",
                "--checkpoint",
                "mx-x-1",
                "mx-x",
            ]
        );
    }

    #[test]
    fn checkpoint_label_is_file_stem() {
        assert_eq!(
            checkpoint_label(Path::new("/snaps/mx-feat-1700.snap")),
            "mx-feat-1700"
        );
    }

    fn cfg(enabled: bool, strategy: CheckpointStrategy) -> CheckpointConfig {
        CheckpointConfig {
            enabled: Some(enabled),
            strategy,
            interval_secs: None,
            dir: None,
            keep: None,
        }
    }

    #[test]
    fn prune_keeps_only_most_recent() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox_id = "mx-test-1234";
        let prefix = format!("{}-", sandbox_id);

        // Create 3 fake snapshot files with distinct timestamps in the name.
        let snaps: Vec<PathBuf> = (1u64..=3)
            .map(|i| {
                let p = dir.path().join(format!("{}{}.snap", prefix, i * 1000));
                std::fs::write(&p, b"snap").unwrap();
                p
            })
            .collect();

        let current = &snaps[2]; // newest
        let mut config = cfg(true, CheckpointStrategy::ModeSwitch);
        config.keep = Some(1);

        prune_old_snapshots(sandbox_id, dir.path(), current, &config);

        // Only the current (newest) snapshot should remain.
        assert!(current.exists(), "current snapshot should be kept");
        assert!(!snaps[0].exists(), "oldest snap should be pruned");
        assert!(!snaps[1].exists(), "middle snap should be pruned");
    }

    #[test]
    fn prune_keeps_n_most_recent() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox_id = "mx-test-5678";
        let prefix = format!("{}-", sandbox_id);

        let snaps: Vec<PathBuf> = (1u64..=4)
            .map(|i| {
                let p = dir.path().join(format!("{}{}.snap", prefix, i * 1000));
                std::fs::write(&p, b"snap").unwrap();
                p
            })
            .collect();

        let current = &snaps[3];
        let mut config = cfg(true, CheckpointStrategy::ModeSwitch);
        config.keep = Some(2);

        prune_old_snapshots(sandbox_id, dir.path(), current, &config);

        // 2 newest should remain, 2 oldest pruned.
        assert!(snaps[3].exists());
        assert!(snaps[2].exists());
        assert!(!snaps[1].exists());
        assert!(!snaps[0].exists());
    }

    #[test]
    fn prune_unlimited_keeps_all() {
        let dir = tempfile::tempdir().unwrap();
        let sandbox_id = "mx-test-unlimited";
        let prefix = format!("{}-", sandbox_id);

        let snaps: Vec<PathBuf> = (1u64..=5)
            .map(|i| {
                let p = dir.path().join(format!("{}{}.snap", prefix, i * 1000));
                std::fs::write(&p, b"snap").unwrap();
                p
            })
            .collect();

        let mut config = cfg(true, CheckpointStrategy::ModeSwitch);
        config.keep = Some(0); // unlimited

        prune_old_snapshots(sandbox_id, dir.path(), &snaps[4], &config);

        for s in &snaps {
            assert!(s.exists(), "all snaps should be kept when keep=0");
        }
    }
}
