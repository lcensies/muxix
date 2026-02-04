//! Lima VM instance management.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::process::Command;

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

/// Lima VM operations.
///
/// This is a thin wrapper around `limactl` commands. VM boot is not performed
/// here -- it is deferred to the tmux pane via the command returned by
/// `wrap_for_lima()`, so the user sees boot progress output directly.
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
