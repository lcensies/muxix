//! OpenCode status tracking setup.
//!
//! Detects OpenCode via the `~/.config/opencode/` directory.
//! Installs plugin by writing `workmux-status.ts` to the plugin directory.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use super::StatusCheck;

/// The OpenCode plugin source, embedded at compile time.
const PLUGIN_SOURCE: &str = include_str!("../../.opencode/plugin/workmux-status.ts");

fn opencode_config_dir() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("opencode"));
    }
    home::home_dir().map(|h| h.join(".config/opencode"))
}

fn plugin_path() -> Option<PathBuf> {
    opencode_config_dir().map(|d| d.join("plugin/workmux-status.ts"))
}

/// Detect if OpenCode is present via filesystem.
/// Returns the reason string if detected, None otherwise.
pub fn detect() -> Option<&'static str> {
    if opencode_config_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.config/opencode/");
    }

    None
}

/// Check if workmux plugin is installed for OpenCode.
pub fn check() -> Result<StatusCheck> {
    let Some(path) = plugin_path() else {
        return Ok(StatusCheck::NotInstalled);
    };

    if path.exists() {
        Ok(StatusCheck::Installed)
    } else {
        Ok(StatusCheck::NotInstalled)
    }
}

/// Install workmux plugin for OpenCode.
/// Returns a description of what was done.
pub fn install() -> Result<String> {
    let path =
        plugin_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create OpenCode plugin directory")?;
    }

    fs::write(&path, PLUGIN_SOURCE).context("Failed to write OpenCode plugin")?;

    Ok(format!(
        "Installed plugin to {}. Restart OpenCode for it to take effect.",
        path.display()
    ))
}
