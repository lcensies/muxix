//! Lima VM instance management.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Command;
use tracing::{debug, info};

use crate::config::Config;

/// Lima instance information from `limactl list --json`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LimaInstanceInfo {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub dir: Option<String>,
}

impl LimaInstanceInfo {
    /// Check if the instance is running.
    pub fn is_running(&self) -> bool {
        self.status == "Running"
    }
}

/// Parse NDJSON output from `limactl list --json` (one JSON object per line).
pub fn parse_lima_instances(stdout: &[u8]) -> Result<Vec<LimaInstanceInfo>> {
    std::str::from_utf8(stdout)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str::<LimaInstanceInfo>(l)
                .with_context(|| format!("Failed to parse limactl row: {}", l))
        })
        .collect()
}

/// VM state detected from `limactl list`.
pub(crate) enum VmState {
    /// VM is already running, no boot needed
    Running,
    /// VM exists but is stopped, needs `limactl start <name>`
    Stopped,
    /// VM doesn't exist, needs `limactl start --name <name> <config>`
    NotFound,
}

/// Check the current state of a Lima VM by name.
pub(crate) fn check_vm_state(vm_name: &str) -> Result<VmState> {
    let instances = LimaInstance::list()?;

    match instances.iter().find(|i| i.name == vm_name) {
        Some(info) if info.is_running() => Ok(VmState::Running),
        Some(_) => Ok(VmState::Stopped),
        None => Ok(VmState::NotFound),
    }
}

/// Lima VM operations.
pub struct LimaInstance;

impl LimaInstance {
    /// Check if limactl is available on the system.
    pub fn is_lima_available() -> bool {
        Command::new("limactl")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// List all Lima instances.
    pub fn list() -> Result<Vec<LimaInstanceInfo>> {
        let output = Command::new("limactl")
            .arg("list")
            .arg("--json")
            .output()
            .context("Failed to list Lima instances")?;

        if !output.status.success() {
            bail!("Failed to list Lima instances");
        }

        parse_lima_instances(&output.stdout)
    }

    /// Stop a Lima VM by name. This is idempotent -- succeeds if the VM is already stopped.
    pub fn stop_by_name(name: &str) -> Result<()> {
        let output = Command::new("limactl")
            .arg("stop")
            .arg(name)
            .output()
            .with_context(|| format!("Failed to execute limactl stop for '{}'", name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Treat "not running" as success for idempotency
            if stderr.contains("not running") {
                return Ok(());
            }
            bail!("Failed to stop Lima VM '{}': {}", name, stderr);
        }

        Ok(())
    }
}

/// Ensure a Lima VM is running for the given worktree.
///
/// Checks the VM state and boots it if necessary, showing a spinner with
/// streaming limactl output in the user's terminal. Should be called from
/// the main process BEFORE creating tmux panes.
///
/// Returns the VM name for use by `wrap_for_lima()`.
pub fn ensure_vm_running(config: &Config, worktree_path: &Path) -> Result<String> {
    if !LimaInstance::is_lima_available() {
        bail!(
            "Lima backend is enabled but limactl is not installed.\n\
             Install Lima: https://lima-vm.io/docs/installation/\n\
             Or disable sandbox: set 'sandbox.enabled: false' in config."
        );
    }

    let isolation = config.sandbox.isolation();
    let vm_name = super::instance_name(worktree_path, isolation.clone(), config)?;

    debug!(vm_name = %vm_name, "checking Lima VM state");
    let vm_state = check_vm_state(&vm_name)?;

    match vm_state {
        VmState::Running => {
            debug!(vm_name = %vm_name, "Lima VM already running");
        }
        VmState::Stopped => {
            info!(vm_name = %vm_name, "starting stopped Lima VM");
            let msg = format!("Starting Lima VM {}", vm_name);
            let mut cmd = Command::new("limactl");
            cmd.args(["start", "--tty=false", "--progress", &vm_name]);

            match crate::spinner::with_streaming_command_formatted(
                &msg,
                cmd,
                super::log_format::format_lima_log_line,
            ) {
                Ok(()) => {}
                Err(_) => {
                    // Race condition: another process may have started the VM.
                    // Re-check state before failing.
                    if matches!(check_vm_state(&vm_name)?, VmState::Running) {
                        return Ok(vm_name);
                    }
                    bail!("Failed to start Lima VM '{}'", vm_name);
                }
            }
        }
        VmState::NotFound => {
            info!(vm_name = %vm_name, "creating new Lima VM");
            // Only generate config and mounts when we need to create a new VM
            let mounts = super::generate_mounts(worktree_path, isolation, config)?;

            eprintln!("  Mounts:");
            for m in &mounts {
                if m.host_path == m.guest_path {
                    eprintln!("    {} (rw)", m.host_path.display());
                } else {
                    eprintln!(
                        "    {} -> {} ({})",
                        m.host_path.display(),
                        m.guest_path.display(),
                        if m.read_only { "ro" } else { "rw" }
                    );
                }
            }

            let lima_config = super::generate_lima_config(&vm_name, &mounts)?;

            let config_path = std::env::temp_dir().join(format!("workmux-lima-{}.yaml", vm_name));
            std::fs::write(&config_path, &lima_config).with_context(|| {
                format!("Failed to write Lima config to {}", config_path.display())
            })?;

            let msg = format!("Creating Lima VM {}", vm_name);
            let mut cmd = Command::new("limactl");
            cmd.args([
                "start",
                "--name",
                &vm_name,
                "--tty=false",
                "--progress",
                &config_path.to_string_lossy(),
            ]);

            match crate::spinner::with_streaming_command_formatted(
                &msg,
                cmd,
                super::log_format::format_lima_log_line,
            ) {
                Ok(()) => {}
                Err(_) => {
                    // Race condition: another process may have created the VM.
                    // Re-check state before failing.
                    if matches!(check_vm_state(&vm_name)?, VmState::Running) {
                        return Ok(vm_name);
                    }
                    bail!("Failed to create Lima VM '{}'", vm_name);
                }
            }
        }
    }

    info!(vm_name = %vm_name, "Lima VM ready");
    Ok(vm_name)
}
