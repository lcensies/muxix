//! Copilot CLI status tracking setup.
//!
//! Detects Copilot CLI via the `~/.copilot/` directory.
//! Installs hooks by writing hooks.json to `.github/hooks/muxix-status/`
//! in the current git repository.
//!
//! Unlike Claude/OpenCode which install globally, Copilot hooks are per-repo.
//! See https://github.com/github/copilot-cli/issues/1157

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

use super::StatusCheck;

/// Hooks configuration embedded at compile time.
const HOOKS_JSON: &str = include_str!("../../../.github/hooks/muxix-status/hooks.json");

/// Copilot CLI's configuration directory, honoring `COPILOT_HOME`.
///
/// `COPILOT_HOME` replaces the whole `~/.copilot` path and carries the config,
/// the customizations (`skills/`, `agents/`, `hooks/`, `copilot-instructions.md`)
/// and the session history with it — which is why profiling Copilot needs no
/// separate session-dir redirect.
pub fn copilot_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("COPILOT_HOME") {
        return Some(PathBuf::from(dir));
    }
    home::home_dir().map(|h| h.join(".copilot"))
}

/// Copilot CLI's own user settings file, honoring `COPILOT_HOME`.
///
/// User-editable settings moved from `config.json` (which is now
/// automatically-managed application state) to `settings.json`; only the
/// latter is safe for a declared `settings:` patch.
pub fn settings_file() -> Option<PathBuf> {
    copilot_home().map(|d| d.join("settings.json"))
}

/// Personal custom instructions, applied to every Copilot CLI session.
pub fn instructions_file() -> Option<PathBuf> {
    copilot_home().map(|d| d.join("copilot-instructions.md"))
}

/// Personal skills root: `<copilot home>/skills/<name>/SKILL.md`.
pub fn skills_dir() -> Option<PathBuf> {
    copilot_home().map(|d| d.join("skills"))
}

/// Personal custom agent definitions: `<copilot home>/agents/<name>.agent.md`.
pub fn subagents_dir() -> Option<PathBuf> {
    copilot_home().map(|d| d.join("agents"))
}

/// Where Copilot CLI keeps user-level declared hooks, and its names for the
/// agent-agnostic events.
///
/// Copilot loads every `*.json` in `$COPILOT_HOME/hooks/`, so muxix owns one
/// file (`muxix.json`) and never edits the user's others. Its dialect differs
/// from the Claude-shaped one: a `version` stamp and flat per-event entries
/// keyed by `bash`.
pub fn declared_hook_target() -> Option<crate::command::setup::agent_hooks::HookTarget> {
    use crate::command::setup::agent_hooks::{HookDialect, HookTarget};
    fn key(event: crate::bootstrap::HookEvent) -> Option<&'static str> {
        use crate::bootstrap::HookEvent;
        match event {
            HookEvent::SessionReady => Some("sessionStart"),
            // `agentStop` fires when the agent finishes responding — Copilot's
            // equivalent of Claude's `Stop`.
            HookEvent::TurnDone => Some("agentStop"),
        }
    }
    Some(HookTarget {
        file: copilot_home()?.join("hooks").join("muxix.json"),
        event_key: key,
        requires_plugin: None,
        dialect: HookDialect::CopilotFlat,
    })
}

/// Copilot CLI instructions bootstrapper: a muxix-managed sentinel region in
/// `$COPILOT_HOME/copilot-instructions.md`, which Copilot applies to every
/// session regardless of the project directory.
pub struct Bootstrapper {
    instructions: PathBuf,
}

impl Bootstrapper {
    pub fn new() -> Option<Self> {
        instructions_file().map(|instructions| Self { instructions })
    }
}

impl super::AgentBootstrapper for Bootstrapper {
    fn instructions_path(&self) -> PathBuf {
        self.instructions.clone()
    }
}

/// Detect if Copilot CLI is present via filesystem.
/// Also requires being in a git repo since hooks are per-repo.
pub fn detect() -> Option<&'static str> {
    if crate::git::get_repo_root().is_err() {
        return None;
    }
    if copilot_home().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.copilot/");
    }
    None
}

/// Check if muxix hooks are installed for Copilot in the current repo.
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
                && content.contains("muxix set-window-status")
            {
                return Ok(StatusCheck::Installed);
            }
        }
    }

    Ok(StatusCheck::NotInstalled)
}

/// Install muxix hooks for Copilot CLI in the current repo.
pub fn install() -> Result<String> {
    let root = crate::git::get_repo_root()
        .context("Must be in a git repository to install Copilot hooks")?;
    let hooks_dir = root.join(".github/hooks/muxix-status");

    fs::create_dir_all(&hooks_dir).context("Failed to create .github/hooks/muxix-status/")?;

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

    use super::super::AgentBootstrapper;

    #[test]
    fn personal_instructions_region_is_managed_and_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("copilot-instructions.md");
        std::fs::write(&path, "my own rules\n").unwrap();
        let b = Bootstrapper {
            instructions: path.clone(),
        };

        b.apply_prompt("declared component").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("my own rules"), "{body}");
        assert_eq!(b.current_prompt().as_deref(), Some("declared component"));

        b.apply_prompt("declared component").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body.matches("muxix-bootstrap-begin").count(), 1, "{body}");
    }

    #[test]
    fn copilot_home_overrides_every_derived_path() {
        // COPILOT_HOME replaces the whole ~/.copilot path, so skills, agents,
        // settings and instructions must all follow it.
        let tmp = tempfile::tempdir().unwrap();
        let prev = std::env::var_os("COPILOT_HOME");
        // SAFETY: single-threaded assertion window; restored below.
        unsafe { std::env::set_var("COPILOT_HOME", tmp.path()) };
        let derived = [
            settings_file().unwrap(),
            instructions_file().unwrap(),
            skills_dir().unwrap(),
            subagents_dir().unwrap(),
        ];
        unsafe {
            match prev {
                Some(v) => std::env::set_var("COPILOT_HOME", v),
                None => std::env::remove_var("COPILOT_HOME"),
            }
        }
        for path in derived {
            assert!(path.starts_with(tmp.path()), "{path:?}");
        }
    }

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
    fn test_hooks_json_contains_muxix_command() {
        assert!(HOOKS_JSON.contains("muxix set-window-status"));
    }
}
