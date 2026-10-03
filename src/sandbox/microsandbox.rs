//! Microsandbox microVM backend (libkrun-based, no TAP networking required).
//!
//! Wraps the `msb` CLI. Each agent runs in an isolated microVM that can be
//! snapshotted and resumed in <100ms. Unlike Firecracker, networking is handled
//! via TSI (Transparent Socket Impersonation) — no root privileges or TAP
//! interface setup needed.
//!
//! CLI reference: https://github.com/microsandbox/microsandbox

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::config::MicroSandboxConfig;

/// microsandbox release this integration targets.
const MSB_VERSION: &str = "0.5.6";

/// Official one-line installer for the `msb` CLI + libkrunfw.
const MSB_INSTALL_URL: &str =
    "https://github.com/superradcompany/microsandbox/releases/download/v0.5.6/install.sh";

/// Resolve the `msb` binary: PATH first, then the locations the installer uses.
///
/// The installer drops `msb` in `~/.local/bin` (or `~/.microsandbox/bin`),
/// which may not be on the current process's PATH, so we check those too.
pub fn msb_bin() -> Option<PathBuf> {
    if let Ok(p) = which::which("msb") {
        return Some(p);
    }
    candidate_install_paths().into_iter().find(|p| p.is_file())
}

fn candidate_install_paths() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(home) = home::home_dir() {
        v.push(home.join(".local/bin/msb"));
        v.push(home.join(".microsandbox/bin/msb"));
    }
    v.push(PathBuf::from("/usr/local/bin/msb"));
    v
}

/// The `msb` binary as a string for embedding in shell command lines (absolute
/// path when resolvable, bare `msb` otherwise so PATH resolution still applies).
fn msb_bin_str() -> String {
    msb_bin()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "msb".to_string())
}

/// Build a `Command` for the resolved `msb` binary.
fn msb_command() -> Command {
    Command::new(msb_bin().unwrap_or_else(|| PathBuf::from("msb")))
}

/// Check whether the `msb` CLI is available (on PATH or in an install dir).
#[allow(dead_code)] // probe API; callers generally use `ensure_installed`
pub fn is_available() -> bool {
    msb_bin().is_some()
}

/// Ensure `msb` is available, auto-installing it to the host if missing.
///
/// Per project decision, the fallback for a missing `msb` is to install it
/// natively (via the upstream `install.sh`) rather than run it inside Docker —
/// libkrun microVMs don't survive an ephemeral container's lifetime. The
/// bundled `docker/Dockerfile.microsandbox` is a packaging artifact only.
pub fn ensure_installed() -> Result<PathBuf> {
    if let Some(p) = msb_bin() {
        return Ok(p);
    }
    install_msb()?;
    msb_bin().ok_or_else(|| {
        anyhow::anyhow!("`msb` still not found after running the microsandbox installer")
    })
}

fn install_msb() -> Result<()> {
    info!(
        version = MSB_VERSION,
        "`msb` not found — installing microsandbox to the host"
    );
    if which::which("curl").is_err() {
        bail!(
            "cannot auto-install microsandbox: `curl` not found. \
             Install msb manually: curl -fsSL {MSB_INSTALL_URL} | sh"
        );
    }
    // curl -fsSL <install.sh> | sh
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("curl -fsSL {MSB_INSTALL_URL} | sh"))
        .status()
        .context("run microsandbox install.sh")?;
    if !status.success() {
        bail!("microsandbox install.sh failed (exit {:?})", status.code());
    }
    Ok(())
}

/// Create and start a named microsandbox VM.
///
/// Returns the sandbox name (same as `name` — callers store this as
/// `AgentState::sandbox_id` for later checkpoint/resume/destroy).
pub fn create(name: &str, config: &MicroSandboxConfig) -> Result<String> {
    let mut cmd = msb_command();
    cmd.args([
        "sandbox",
        "create",
        name,
        "--image",
        config.resolved_image(),
    ]);

    cmd.args(["--cpus", &config.cpus().to_string()]);

    if let Some(ref mem) = config.memory {
        cmd.args(["--memory", mem]);
    }

    if let Some(ref disk) = config.disk {
        cmd.args(["--disk", disk]);
    }

    let status = cmd.status().context("failed to run `msb sandbox create`")?;

    if !status.success() {
        bail!("msb sandbox create failed for '{}'", name);
    }

    Ok(name.to_string())
}

/// Mount a host directory into a running sandbox VM.
pub fn mount(name: &str, host_path: &Path, guest_path: &str) -> Result<()> {
    let status = msb_command()
        .args([
            "sandbox",
            "mount",
            name,
            &host_path.to_string_lossy(),
            guest_path,
        ])
        .status()
        .context("failed to run `msb sandbox mount`")?;

    if !status.success() {
        bail!("msb sandbox mount failed for '{}'", name);
    }
    Ok(())
}

/// Execute a command inside a sandbox VM, attaching stdio.
///
/// Returns the process exit code.
pub fn exec(name: &str, command: &[&str]) -> Result<i32> {
    let mut cmd = msb_command();
    cmd.args(["sandbox", "exec", name, "--"]);
    cmd.args(command);

    let status = cmd.status().context("failed to run `msb sandbox exec`")?;

    Ok(status.code().unwrap_or(1))
}

/// Run an interactive shell inside a sandbox VM (attaches to stdio).
pub fn shell(name: &str) -> Result<i32> {
    exec(name, &["bash", "-l"])
}

/// Snapshot a running sandbox to disk for later restore.
///
/// `snapshot_path` should be a file path (e.g. `checkpoints/vm-name.snap`).
pub fn snapshot(name: &str, snapshot_path: &Path) -> Result<()> {
    if let Some(parent) = snapshot_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create checkpoint dir {}", parent.display()))?;
    }

    let status = msb_command()
        .args([
            "sandbox",
            "snapshot",
            name,
            "--output",
            &snapshot_path.to_string_lossy(),
        ])
        .status()
        .context("failed to run `msb sandbox snapshot`")?;

    if !status.success() {
        bail!("msb sandbox snapshot failed for '{}'", name);
    }
    Ok(())
}

/// Restore a sandbox from a previously saved snapshot.
pub fn restore(name: &str, snapshot_path: &Path) -> Result<()> {
    if !snapshot_path.exists() {
        bail!("snapshot not found: {}", snapshot_path.display());
    }

    let status = msb_command()
        .args([
            "sandbox",
            "restore",
            name,
            "--from",
            &snapshot_path.to_string_lossy(),
        ])
        .status()
        .context("failed to run `msb sandbox restore`")?;

    if !status.success() {
        bail!("msb sandbox restore failed for '{}'", name);
    }
    Ok(())
}

/// Stop a running sandbox VM (preserves its disk state).
pub fn stop(name: &str) -> Result<()> {
    let status = msb_command()
        .args(["sandbox", "stop", name])
        .status()
        .context("failed to run `msb sandbox stop`")?;

    if !status.success() {
        bail!("msb sandbox stop failed for '{}'", name);
    }
    Ok(())
}

/// Destroy a sandbox VM, freeing all resources.
#[allow(dead_code)] // lifecycle API; used by worktree-removal cleanup
pub fn destroy(name: &str) -> Result<()> {
    let status = msb_command()
        .args(["sandbox", "destroy", name])
        .status()
        .context("failed to run `msb sandbox destroy`")?;

    if !status.success() {
        bail!("msb sandbox destroy failed for '{}'", name);
    }
    Ok(())
}

/// List all microsandbox VMs whose names start with the given prefix.
#[allow(dead_code)] // lifecycle API; used for sandbox discovery/cleanup
pub fn list_with_prefix(prefix: &str) -> Result<Vec<String>> {
    let output = msb_command()
        .args(["sandbox", "list", "--json"])
        .output()
        .context("failed to run `msb sandbox list`")?;

    if !output.status.success() {
        bail!("msb sandbox list failed");
    }

    let text = String::from_utf8_lossy(&output.stdout);
    // Each line is a sandbox name (or JSON — parse conservatively)
    let names: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let name = line.trim().trim_matches('"').trim_matches(',');
            if name.starts_with(prefix) {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect();

    Ok(names)
}

/// Build the *stable* sandbox name for a (worktree, agent) pair.
///
/// Format: `wm-<worktree-handle>-<agent-slug>`.
///
/// The identity is intentionally stable across launches — it does NOT include
/// the process id. This is what makes checkpoint/resume meaningful: re-opening
/// the same worktree+agent addresses the same logical VM, so a prior snapshot
/// can be restored into it. (The previous pid-suffixed scheme produced a brand
/// new name every launch, which could never match an existing checkpoint.)
///
/// Different agents sharing one pane get *different* sandbox names, so swapping
/// agents in a pane checkpoints one VM and resumes the other.
pub fn sandbox_name(worktree_handle: &str, agent: &str) -> String {
    format!("wm-{}-{}", slug(worktree_handle), slug(agent))
}

/// Slugify a component for use in a sandbox name: lowercase alnum, others → `-`.
fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

/// Build a self-contained shell command that brings a sandbox up and runs
/// `inner_cmd` inside it, suitable for a tmux `send-keys` / pane command.
///
/// - `restore_from: Some(path)` restores the VM from a snapshot first.
/// - `restore_from: None` creates the VM (idempotently) and mounts the worktree.
///
/// In both cases the worktree is available read-write at its host path and the
/// inner command runs with that as the working directory. `create` is guarded
/// with `|| true` so re-launching an already-existing sandbox is not fatal.
pub fn launch_command(
    name: &str,
    workdir: &Path,
    inner_cmd: &str,
    restore_from: Option<&Path>,
    config: &MicroSandboxConfig,
) -> String {
    // Use the resolved binary path so the command works in a pane whose PATH
    // doesn't include the installer's ~/.local/bin.
    let msb = msb_bin_str();
    let q_name = shell_quote(name);
    let q_wd = shell_quote(&workdir.to_string_lossy());
    let exec = format!(
        "{msb} sandbox exec {q_name} -- bash -c {}",
        shell_quote(&format!("cd {q_wd} && {inner_cmd}")),
    );

    if let Some(snap) = restore_from {
        // Restore brings back the full VM state (including mounts captured in
        // the snapshot); we still re-assert the mount to be safe.
        format!(
            "{msb} sandbox restore {q_name} --from {snap} && \
             {msb} sandbox mount {q_name} {q_wd} {q_wd} 2>/dev/null; {exec}",
            snap = shell_quote(&snap.to_string_lossy()),
        )
    } else {
        let mut create = format!(
            "{msb} sandbox create {q_name} --image {} --cpus {}",
            shell_quote(config.resolved_image()),
            config.cpus(),
        );
        if let Some(ref mem) = config.memory {
            create.push_str(&format!(" --memory {}", shell_quote(mem)));
        }
        if let Some(ref disk) = config.disk {
            create.push_str(&format!(" --disk {}", shell_quote(disk)));
        }
        format!(
            "{{ {create}; }} 2>/dev/null || true; \
             {msb} sandbox mount {q_name} {q_wd} {q_wd} 2>/dev/null; {exec}",
        )
    }
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn sandbox_name_is_stable_and_agent_scoped() {
        // No pid component: two calls produce the same name (resume can match).
        assert_eq!(
            sandbox_name("my-feature", "claude"),
            sandbox_name("my-feature", "claude")
        );
        assert_eq!(sandbox_name("my-feature", "claude"), "wm-my-feature-claude");
        // Different agents in the same worktree get distinct sandboxes.
        assert_ne!(
            sandbox_name("my-feature", "claude"),
            sandbox_name("my-feature", "opencode")
        );
    }

    #[test]
    fn slug_normalises_unsafe_chars() {
        assert_eq!(slug("Feature/Branch_1"), "feature-branch-1");
        assert_eq!(slug("--weird--"), "weird");
    }

    #[test]
    fn launch_command_create_branch_mounts_and_execs() {
        let cfg = MicroSandboxConfig::default();
        let cmd = launch_command(
            "wm-proj-claude",
            &PathBuf::from("/work/proj"),
            "claude --yolo",
            None,
            &cfg,
        );
        assert!(cmd.contains("msb sandbox create 'wm-proj-claude'"), "{cmd}");
        assert!(
            cmd.contains("msb sandbox mount 'wm-proj-claude' '/work/proj' '/work/proj'"),
            "{cmd}"
        );
        assert!(
            cmd.contains("msb sandbox exec 'wm-proj-claude' -- bash -c"),
            "{cmd}"
        );
        // The inner command is single-quoted for `bash -c`, so the workdir's
        // own quotes are escaped as '\'' — assert on the stable fragments.
        assert!(cmd.contains("claude --yolo"), "{cmd}");
        assert!(cmd.contains("cd "), "{cmd}");
        // Create must not be fatal when the sandbox already exists.
        assert!(cmd.contains("|| true"), "{cmd}");
    }

    #[test]
    fn launch_command_restore_branch_uses_snapshot() {
        let cfg = MicroSandboxConfig::default();
        let snap = PathBuf::from("/snaps/wm-proj-claude-123.snap");
        let cmd = launch_command(
            "wm-proj-claude",
            &PathBuf::from("/work/proj"),
            "claude",
            Some(&snap),
            &cfg,
        );
        assert!(
            cmd.contains(
                "msb sandbox restore 'wm-proj-claude' --from '/snaps/wm-proj-claude-123.snap'"
            ),
            "{cmd}"
        );
        assert!(
            !cmd.contains("msb sandbox create"),
            "restore branch must not create: {cmd}"
        );
        assert!(cmd.contains("msb sandbox exec 'wm-proj-claude'"), "{cmd}");
    }
}
