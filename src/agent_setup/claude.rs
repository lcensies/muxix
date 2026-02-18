//! Claude Code status tracking setup.
//!
//! Detects Claude Code via the `~/.claude/` directory.
//! Installs hooks by merging into `~/.claude/settings.json`.

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use super::StatusCheck;

/// Hooks extracted from `.claude-plugin/plugin.json` at compile time.
const PLUGIN_JSON: &str = include_str!("../../.claude-plugin/plugin.json");

fn settings_path() -> Option<PathBuf> {
    home::home_dir().map(|h| h.join(".claude/settings.json"))
}

fn claude_dir() -> Option<PathBuf> {
    home::home_dir().map(|h| h.join(".claude"))
}

/// Detect if Claude Code is present via filesystem.
/// Returns the reason string if detected, None otherwise.
pub fn detect() -> Option<&'static str> {
    if claude_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.claude/");
    }

    None
}

/// Check if workmux hooks are installed in Claude Code settings.
///
/// Checks two paths:
/// 1. Plugin: `enabledPlugins` has a key starting with `workmux-status@`
///    (regardless of enabled/disabled -- user knows about it)
/// 2. Manual hooks: `hooks` object contains a command with `workmux set-window-status`
pub fn check() -> Result<StatusCheck> {
    let Some(path) = settings_path() else {
        return Ok(StatusCheck::NotInstalled);
    };

    if !path.exists() {
        return Ok(StatusCheck::NotInstalled);
    }

    let content = fs::read_to_string(&path).context("Failed to read ~/.claude/settings.json")?;

    let settings: Value =
        serde_json::from_str(&content).context("~/.claude/settings.json is not valid JSON")?;

    Ok(check_settings(&settings))
}

/// Check a parsed settings.json value for workmux status tracking configuration.
fn check_settings(settings: &Value) -> StatusCheck {
    // Check for plugin installation
    if let Some(plugins) = settings.get("enabledPlugins").and_then(|v| v.as_object())
        && plugins.keys().any(|k| k.starts_with("workmux-status@"))
    {
        return StatusCheck::Installed;
    }

    // Check for manual hooks by traversing the hooks structure
    if has_workmux_hooks(settings) {
        return StatusCheck::Installed;
    }

    StatusCheck::NotInstalled
}

/// Check if the hooks object contains any workmux set-window-status commands.
fn has_workmux_hooks(settings: &Value) -> bool {
    let Some(hooks) = settings.get("hooks").and_then(|v| v.as_object()) else {
        return false;
    };

    for (_event, groups) in hooks {
        let Some(groups_arr) = groups.as_array() else {
            continue;
        };
        for group in groups_arr {
            let Some(hook_list) = group.get("hooks").and_then(|v| v.as_array()) else {
                continue;
            };
            for hook in hook_list {
                if let Some(cmd) = hook.get("command").and_then(|v| v.as_str())
                    && cmd.contains("workmux set-window-status")
                {
                    return true;
                }
            }
        }
    }

    false
}

/// Extract the hooks object from the plugin.json manifest.
fn load_hooks_from_plugin() -> Result<Value> {
    let plugin: Value =
        serde_json::from_str(PLUGIN_JSON).expect("embedded plugin.json is valid JSON");
    plugin
        .get("hooks")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("plugin.json missing hooks key"))
}

/// Install workmux hooks into `~/.claude/settings.json`.
///
/// Merges hook groups into existing hooks without clobbering or creating
/// duplicates. Returns a description of what was done.
pub fn install() -> Result<String> {
    let path =
        settings_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    // Read existing settings or start fresh
    let mut settings: Value = if path.exists() {
        let content =
            fs::read_to_string(&path).context("Failed to read ~/.claude/settings.json")?;
        serde_json::from_str(&content).context("~/.claude/settings.json is not valid JSON")?
    } else {
        // Ensure ~/.claude/ directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create ~/.claude/ directory")?;
        }
        Value::Object(serde_json::Map::new())
    };

    let hooks_to_add = load_hooks_from_plugin()?;

    // Ensure settings.hooks exists as an object
    let settings_obj = settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("settings.json root is not an object"))?;

    if !settings_obj.contains_key("hooks") {
        settings_obj.insert("hooks".to_string(), Value::Object(serde_json::Map::new()));
    }

    let existing_hooks = settings_obj
        .get_mut("hooks")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("settings.json hooks is not an object"))?;

    // Merge each hook event, deduplicating by value equality
    let hooks_map = hooks_to_add.as_object().expect("plugin hooks is an object");
    for (event, hook_groups) in hooks_map {
        let Some(new_groups) = hook_groups.as_array() else {
            continue;
        };

        if let Some(existing_groups) = existing_hooks.get_mut(event) {
            let arr = existing_groups
                .as_array_mut()
                .ok_or_else(|| anyhow::anyhow!("hooks.{event} is not an array"))?;
            for group in new_groups {
                if !arr.contains(group) {
                    arr.push(group.clone());
                }
            }
        } else {
            existing_hooks.insert(event.clone(), hook_groups.clone());
        }
    }

    // Write back with pretty formatting
    let output = serde_json::to_string_pretty(&settings)?;
    fs::write(&path, output + "\n").context("Failed to write ~/.claude/settings.json")?;

    Ok("Installed hooks to ~/.claude/settings.json".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_has_workmux_hooks_empty() {
        let settings = json!({});
        assert!(!has_workmux_hooks(&settings));
    }

    #[test]
    fn test_has_workmux_hooks_present() {
        let settings = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "workmux set-window-status done"
                    }]
                }]
            }
        });
        assert!(has_workmux_hooks(&settings));
    }

    #[test]
    fn test_has_workmux_hooks_other_hooks_only() {
        let settings = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "afplay /System/Library/Sounds/Glass.aiff"
                    }]
                }]
            }
        });
        assert!(!has_workmux_hooks(&settings));
    }

    #[test]
    fn test_load_hooks_from_plugin() {
        let hooks = load_hooks_from_plugin().unwrap();
        let obj = hooks.as_object().unwrap();
        assert!(obj.contains_key("UserPromptSubmit"));
        assert!(obj.contains_key("Notification"));
        assert!(obj.contains_key("PostToolUse"));
        assert!(obj.contains_key("Stop"));
    }

    #[test]
    fn test_merge_into_empty_settings() {
        let mut settings = json!({});
        let hooks_to_add = load_hooks_from_plugin().unwrap();
        let hooks_map = hooks_to_add.as_object().unwrap();

        let settings_obj = settings.as_object_mut().unwrap();
        settings_obj.insert("hooks".to_string(), Value::Object(serde_json::Map::new()));
        let existing_hooks = settings_obj
            .get_mut("hooks")
            .unwrap()
            .as_object_mut()
            .unwrap();

        for (event, hook_groups) in hooks_map {
            existing_hooks.insert(event.clone(), hook_groups.clone());
        }

        let hooks = settings.get("hooks").unwrap().as_object().unwrap();
        assert_eq!(hooks.len(), 4);
    }

    #[test]
    fn test_merge_deduplicates() {
        let mut settings = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "workmux set-window-status done"
                    }]
                }]
            }
        });

        let hooks_to_add = load_hooks_from_plugin().unwrap();
        let hooks_map = hooks_to_add.as_object().unwrap();

        let existing_hooks = settings.get_mut("hooks").unwrap().as_object_mut().unwrap();

        for (event, hook_groups) in hooks_map {
            let new_groups = hook_groups.as_array().unwrap();
            if let Some(existing_groups) = existing_hooks.get_mut(event) {
                let arr = existing_groups.as_array_mut().unwrap();
                for group in new_groups {
                    if !arr.contains(group) {
                        arr.push(group.clone());
                    }
                }
            } else {
                existing_hooks.insert(event.clone(), hook_groups.clone());
            }
        }

        // Stop should still have exactly 1 group (not duplicated)
        let stop = settings
            .get("hooks")
            .unwrap()
            .get("Stop")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(stop.len(), 1);
    }

    #[test]
    fn test_check_settings_empty() {
        let settings = json!({});
        assert!(matches!(
            check_settings(&settings),
            StatusCheck::NotInstalled
        ));
    }

    #[test]
    fn test_check_settings_plugin_enabled() {
        let settings = json!({
            "enabledPlugins": {
                "workmux-status@workmux": true
            }
        });
        assert!(matches!(check_settings(&settings), StatusCheck::Installed));
    }

    #[test]
    fn test_check_settings_plugin_disabled() {
        let settings = json!({
            "enabledPlugins": {
                "workmux-status@workmux": false
            }
        });
        assert!(matches!(check_settings(&settings), StatusCheck::Installed));
    }

    #[test]
    fn test_check_settings_plugin_different_version() {
        let settings = json!({
            "enabledPlugins": {
                "workmux-status@1.2.3": true
            }
        });
        assert!(matches!(check_settings(&settings), StatusCheck::Installed));
    }

    #[test]
    fn test_check_settings_other_plugins_only() {
        let settings = json!({
            "enabledPlugins": {
                "some-other-plugin@1.0": true
            }
        });
        assert!(matches!(
            check_settings(&settings),
            StatusCheck::NotInstalled
        ));
    }

    #[test]
    fn test_check_settings_hooks_installed() {
        let settings = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "workmux set-window-status done"
                    }]
                }]
            }
        });
        assert!(matches!(check_settings(&settings), StatusCheck::Installed));
    }

    #[test]
    fn test_check_settings_both_plugin_and_hooks() {
        let settings = json!({
            "enabledPlugins": {
                "workmux-status@workmux": true
            },
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "workmux set-window-status done"
                    }]
                }]
            }
        });
        assert!(matches!(check_settings(&settings), StatusCheck::Installed));
    }

    #[test]
    fn test_merge_preserves_existing_hooks() {
        let mut settings = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "afplay /System/Library/Sounds/Glass.aiff"
                    }]
                }]
            }
        });

        let hooks_to_add = load_hooks_from_plugin().unwrap();
        let hooks_map = hooks_to_add.as_object().unwrap();

        let existing_hooks = settings.get_mut("hooks").unwrap().as_object_mut().unwrap();

        for (event, hook_groups) in hooks_map {
            let new_groups = hook_groups.as_array().unwrap();
            if let Some(existing_groups) = existing_hooks.get_mut(event) {
                let arr = existing_groups.as_array_mut().unwrap();
                for group in new_groups {
                    if !arr.contains(group) {
                        arr.push(group.clone());
                    }
                }
            } else {
                existing_hooks.insert(event.clone(), hook_groups.clone());
            }
        }

        // Stop should have 2 groups (original afplay + workmux)
        let stop = settings
            .get("hooks")
            .unwrap()
            .get("Stop")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(stop.len(), 2);

        // All 4 events should be present
        let hooks = settings.get("hooks").unwrap().as_object().unwrap();
        assert_eq!(hooks.len(), 4);
    }
}
