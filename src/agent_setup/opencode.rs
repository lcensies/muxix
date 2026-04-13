//! OpenCode status tracking setup.
//!
//! Detects OpenCode via its config directory. Resolution order:
//! 1. `OPENCODE_CONFIG` env var (explicit override)
//! 2. `XDG_CONFIG_HOME/opencode`
//! 3. `~/.config/opencode`
//!
//! Installs plugin by writing `package.json` and `workmux-status.ts` to the
//! OpenCode config directory.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use super::StatusCheck;

/// OpenCode distribution files, embedded at compile time.
const PLUGIN_SOURCE: &str = include_str!("../../resources/opencode/plugins/workmux-status.ts");
const PACKAGE_JSON: &str = include_str!("../../resources/opencode/package.json");

pub fn opencode_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("OPENCODE_CONFIG") {
        return Some(PathBuf::from(dir));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("opencode"));
    }
    home::home_dir().map(|h| h.join(".config/opencode"))
}

fn plugin_path() -> Option<PathBuf> {
    opencode_config_dir().map(|d| d.join("plugins/workmux-status.ts"))
}

fn legacy_plugin_path() -> Option<PathBuf> {
    opencode_config_dir().map(|d| d.join("plugin/workmux-status.ts"))
}

fn package_json_path() -> Option<PathBuf> {
    opencode_config_dir().map(|d| d.join("package.json"))
}

/// Detect if OpenCode is present via filesystem.
/// Returns the reason string if detected, None otherwise.
pub fn detect() -> Option<&'static str> {
    if std::env::var("OPENCODE_CONFIG").is_ok_and(|d| PathBuf::from(d).is_dir()) {
        return Some("found $OPENCODE_CONFIG");
    }
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

    if path.exists() || legacy_plugin_path().is_some_and(|legacy| legacy.exists()) {
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
    let package_json =
        package_json_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create OpenCode plugin directory")?;
    }

    if let Some(parent) = package_json.parent() {
        fs::create_dir_all(parent).context("Failed to create OpenCode config directory")?;
    }

    fs::write(&package_json, PACKAGE_JSON).context("Failed to write OpenCode package.json")?;
    fs::write(&path, PLUGIN_SOURCE).context("Failed to write OpenCode plugin")?;

    Ok(format!(
        "Installed OpenCode plugin files to {} and {}. Restart OpenCode for it to take effect.",
        package_json.display(),
        path.display()
    ))
}
