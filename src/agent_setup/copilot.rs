//! Copilot CLI status tracking setup.
//!
//! Detects Copilot CLI via the `~/.copilot/` directory.
//! Installs hooks by writing hooks.json to `.github/hooks/workmux-status/`
//! in the current git repository.
//!
//! Unlike Claude/OpenCode which install globally, Copilot hooks are per-repo.
//! See https://github.com/github/copilot-cli/issues/1157

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use super::StatusCheck;

/// Hooks configuration embedded at compile time.
const HOOKS_JSON: &str = include_str!("../../.github/hooks/workmux-status/hooks.json");

fn copilot_dir() -> Option<PathBuf> {
    home::home_dir().map(|h| h.join(".copilot"))
}

/// Detect if Copilot CLI is present via filesystem.
/// Also requires being in a git repo since hooks are per-repo.
pub fn detect() -> Option<&'static str> {
    if crate::git::get_repo_root().is_err() {
        return None;
    }
    if copilot_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.copilot/");
    }
    None
}

/// Check if workmux hooks are installed for Copilot in the current repo.
pub fn check() -> Result<StatusCheck> {
    let root = match crate::git::get_repo_root() {
        Ok(r) => r,
        Err(e) => return Ok(StatusCheck::Error(e.to_string())),
    };

    let hooks_dir = root.join(".github/hooks");
    if !hooks_dir.is_dir() {
        return Ok(StatusCheck::NotInstalled);
    }

    // Scan all hooks.json files under .github/hooks/*/
    if let Ok(entries) = fs::read_dir(&hooks_dir) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let hooks_file = entry.path().join("hooks.json");
            if hooks_file.exists()
                && let Ok(content) = fs::read_to_string(&hooks_file)
                && content.contains("workmux set-window-status")
            {
                return Ok(StatusCheck::Installed);
            }
        }
    }

    Ok(StatusCheck::NotInstalled)
}

/// Install workmux hooks for Copilot CLI in the current repo.
pub fn install() -> Result<String> {
    let root = crate::git::get_repo_root()
        .context("Must be in a git repository to install Copilot hooks")?;
    let hooks_dir = root.join(".github/hooks/workmux-status");

    fs::create_dir_all(&hooks_dir).context("Failed to create .github/hooks/workmux-status/")?;

    let hooks_file = hooks_dir.join("hooks.json");
    fs::write(&hooks_file, HOOKS_JSON).context("Failed to write hooks.json")?;

    Ok(format!(
        "Installed hooks to {}",
        hooks_file
            .strip_prefix(&root)
            .unwrap_or(&hooks_file)
            .display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hooks_json_is_valid() {
        let parsed: serde_json::Value =
            serde_json::from_str(HOOKS_JSON).expect("embedded hooks.json is valid JSON");
        assert_eq!(parsed.get("version").and_then(|v| v.as_u64()), Some(1));
        let hooks = parsed.get("hooks").unwrap().as_object().unwrap();
        assert!(hooks.contains_key("userPromptSubmitted"));
        assert!(hooks.contains_key("postToolUse"));
        assert!(hooks.contains_key("agentStop"));
    }

    #[test]
    fn test_hooks_json_contains_workmux_command() {
        assert!(HOOKS_JSON.contains("workmux set-window-status"));
    }
}
