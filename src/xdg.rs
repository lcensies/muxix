//! Centralized XDG Base Directory resolution.
//!
//! All workmux-owned paths should go through this module rather than
//! resolving `home::home_dir()` directly.

use anyhow::{Context, Result};
use std::path::PathBuf;

/// Resolve an XDG base directory.
///
/// Checks the given env var first (empty or relative values are treated as
/// unset per the XDG spec), then falls back to `$HOME/<default_suffix>`.
fn base_dir(env_var: &str, default_suffix: &str) -> Result<PathBuf> {
    if let Some(val) = std::env::var_os(env_var).filter(|v| !v.is_empty()) {
        let path = PathBuf::from(val);
        if path.is_absolute() {
            return Ok(path);
        }
        tracing::debug!(
            var = env_var,
            "ignoring non-absolute XDG path, falling back to default"
        );
    }
    let home = home::home_dir().context("Could not determine home directory")?;
    Ok(home.join(default_suffix))
}

/// `$XDG_CONFIG_HOME/workmux` (default: `~/.config/workmux`)
pub fn config_dir() -> Result<PathBuf> {
    Ok(base_dir("XDG_CONFIG_HOME", ".config")?.join("workmux"))
}

/// `$XDG_CACHE_HOME/workmux` (default: `~/.cache/workmux`)
pub fn cache_dir() -> Result<PathBuf> {
    Ok(base_dir("XDG_CACHE_HOME", ".cache")?.join("workmux"))
}

/// `$XDG_STATE_HOME/workmux` (default: `~/.local/state/workmux`)
pub fn state_dir() -> Result<PathBuf> {
    Ok(base_dir("XDG_STATE_HOME", ".local/state")?.join("workmux"))
}
