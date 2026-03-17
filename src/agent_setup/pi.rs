//! Pi agent status tracking setup.
//!
//! Detects pi via its config directory at `~/.pi/agent/`.
//! Override with `PI_CODING_AGENT_DIR` env var.
//!
//! Installs extension by writing `workmux-status.ts` to the extensions directory.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use super::StatusCheck;

/// The pi extension source, embedded at compile time.
const EXTENSION_SOURCE: &str = include_str!("../../.pi/extensions/workmux-status.ts");

fn pi_agent_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        return Some(PathBuf::from(dir));
    }
    home::home_dir().map(|h| h.join(".pi/agent"))
}

fn extension_path() -> Option<PathBuf> {
    pi_agent_dir().map(|d| d.join("extensions/workmux-status.ts"))
}

/// Detect if pi is present via filesystem.
/// Returns the reason string if detected, None otherwise.
pub fn detect() -> Option<&'static str> {
    if std::env::var("PI_CODING_AGENT_DIR").is_ok_and(|d| PathBuf::from(d).is_dir()) {
        return Some("found $PI_CODING_AGENT_DIR");
    }
    if pi_agent_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.pi/agent/");
    }
    None
}

/// Check if workmux extension is installed for pi.
pub fn check() -> Result<StatusCheck> {
    let Some(path) = extension_path() else {
        return Ok(StatusCheck::NotInstalled);
    };

    if path.exists() {
        Ok(StatusCheck::Installed)
    } else {
        Ok(StatusCheck::NotInstalled)
    }
}

/// Install workmux extension for pi.
/// Returns a description of what was done.
pub fn install() -> Result<String> {
    let path =
        extension_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create pi extensions directory")?;
    }

    fs::write(&path, EXTENSION_SOURCE).context("Failed to write pi extension")?;

    Ok(format!(
        "Installed extension to {}. Restart pi for it to take effect.",
        path.display()
    ))
}
