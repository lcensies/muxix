//! Gemini CLI status tracking setup.
//!
//! Detects Gemini CLI via the `~/.gemini/` directory.
//! Installs hooks by merging into `~/.gemini/settings.json`.

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use super::StatusCheck;

/// Hooks configuration embedded at compile time.
const HOOKS_JSON: &str = include_str!("../../resources/gemini/settings.json");

fn gemini_dir() -> Option<PathBuf> {
    home::home_dir().map(|h| h.join(".gemini"))
}

fn settings_path() -> Option<PathBuf> {
    gemini_dir().map(|d| d.join("settings.json"))
}

/// Detect if Gemini CLI is present via filesystem.
pub fn detect() -> Option<&'static str> {
    if gemini_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.gemini/");
    }
    None
}

/// Check if workmux hooks are installed in Gemini settings.json.
pub fn check() -> Result<StatusCheck> {
    let Some(path) = settings_path() else {
        return Ok(StatusCheck::NotInstalled);
    };

    if !path.exists() {
        return Ok(StatusCheck::NotInstalled);
    }

    let content = fs::read_to_string(&path).context("Failed to read ~/.gemini/settings.json")?;
    let config: Value =
        serde_json::from_str(&content).context("~/.gemini/settings.json is not valid JSON")?;

    if has_workmux_hooks(&config) {
        Ok(StatusCheck::Installed)
    } else {
        Ok(StatusCheck::NotInstalled)
    }
}

/// Check if the hooks object contains any workmux set-window-status commands.
fn has_workmux_hooks(config: &Value) -> bool {
    let Some(hooks) = config.get("hooks").and_then(|v| v.as_object()) else {
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

/// Load the hooks portion from the embedded config.
fn load_hooks() -> Result<Value> {
    let config: Value =
        serde_json::from_str(HOOKS_JSON).expect("embedded hooks config is valid JSON");
    config
        .get("hooks")
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("hooks config missing hooks key"))
}

/// Install workmux hooks into `~/.gemini/settings.json`.
///
/// Merges hook groups into existing hooks without clobbering or creating
/// duplicates. Returns a description of what was done.
pub fn install() -> Result<String> {
    let path =
        settings_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    // Read existing config or start fresh
    let mut config: Value = if path.exists() {
        let content =
            fs::read_to_string(&path).context("Failed to read ~/.gemini/settings.json")?;
        serde_json::from_str(&content).context("~/.gemini/settings.json is not valid JSON")?
    } else {
        // Ensure ~/.gemini/ directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create ~/.gemini/ directory")?;
        }
        serde_json::json!({ "hooks": {} })
    };

    let hooks_to_add = load_hooks()?;

    // Ensure config.hooks exists as an object
    let config_obj = config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("settings.json root is not an object"))?;

    if !config_obj.contains_key("hooks") {
        config_obj.insert("hooks".to_string(), Value::Object(serde_json::Map::new()));
    }

    let existing_hooks = config_obj
        .get_mut("hooks")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("settings.json hooks is not an object"))?;

    // Merge each hook event, deduplicating by value equality
    let hooks_map = hooks_to_add.as_object().expect("hooks is an object");
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
    let output = serde_json::to_string_pretty(&config)?;
    fs::write(&path, output + "\n").context("Failed to write ~/.gemini/settings.json")?;

    Ok("Installed hooks to ~/.gemini/settings.json".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_hooks_json_is_valid() {
        let parsed: serde_json::Value =
            serde_json::from_str(HOOKS_JSON).expect("embedded hooks config is valid JSON");
        let hooks = parsed.get("hooks").unwrap().as_object().unwrap();
        assert!(hooks.contains_key("BeforeAgent"));
        assert!(hooks.contains_key("Notification"));
        assert!(hooks.contains_key("AfterTool"));
        assert!(hooks.contains_key("AfterAgent"));
        assert!(hooks.contains_key("SessionEnd"));
    }

    #[test]
    fn test_hooks_json_contains_workmux_command() {
        assert!(HOOKS_JSON.contains("workmux set-window-status"));
    }

    #[test]
    fn test_has_workmux_hooks_empty() {
        let config = json!({});
        assert!(!has_workmux_hooks(&config));
    }

    #[test]
    fn test_has_workmux_hooks_present() {
        let config = json!({
            "hooks": {
                "AfterAgent": [{
                    "hooks": [{
                        "type": "command",
                        "command": "workmux set-window-status done"
                    }]
                }]
            }
        });
        assert!(has_workmux_hooks(&config));
    }

    #[test]
    fn test_has_workmux_hooks_other_hooks_only() {
        let config = json!({
            "hooks": {
                "AfterAgent": [{
                    "hooks": [{
                        "type": "command",
                        "command": "python3 my-script.py"
                    }]
                }]
            }
        });
        assert!(!has_workmux_hooks(&config));
    }

    #[test]
    fn test_load_hooks() {
        let hooks = load_hooks().unwrap();
        let obj = hooks.as_object().unwrap();
        assert!(obj.contains_key("BeforeAgent"));
        assert!(obj.contains_key("Notification"));
        assert!(obj.contains_key("AfterTool"));
        assert!(obj.contains_key("AfterAgent"));
        assert!(obj.contains_key("SessionEnd"));
    }

    #[test]
    fn test_merge_into_empty_config() {
        let mut config = json!({ "hooks": {} });
        let hooks_to_add = load_hooks().unwrap();
        let hooks_map = hooks_to_add.as_object().unwrap();

        let existing_hooks = config.get_mut("hooks").unwrap().as_object_mut().unwrap();

        for (event, hook_groups) in hooks_map {
            existing_hooks.insert(event.clone(), hook_groups.clone());
        }

        let hooks = config.get("hooks").unwrap().as_object().unwrap();
        assert_eq!(hooks.len(), 5);
    }

    #[test]
    fn test_merge_deduplicates() {
        let mut config = json!({
            "hooks": {
                "AfterAgent": [{
                    "hooks": [{
                        "type": "command",
                        "command": "workmux set-window-status done"
                    }]
                }]
            }
        });

        let hooks_to_add = load_hooks().unwrap();
        let hooks_map = hooks_to_add.as_object().unwrap();

        let existing_hooks = config.get_mut("hooks").unwrap().as_object_mut().unwrap();

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

        // AfterAgent should still have exactly 1 group (not duplicated)
        let after_agent = config
            .get("hooks")
            .unwrap()
            .get("AfterAgent")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(after_agent.len(), 1);
    }

    #[test]
    fn test_merge_preserves_existing_hooks() {
        let mut config = json!({
            "hooks": {
                "AfterAgent": [{
                    "hooks": [{
                        "type": "command",
                        "command": "python3 my-after-hook.py"
                    }]
                }]
            }
        });

        let hooks_to_add = load_hooks().unwrap();
        let hooks_map = hooks_to_add.as_object().unwrap();

        let existing_hooks = config.get_mut("hooks").unwrap().as_object_mut().unwrap();

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

        // AfterAgent should have 2 groups (original + workmux)
        let after_agent = config
            .get("hooks")
            .unwrap()
            .get("AfterAgent")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(after_agent.len(), 2);

        // All 5 events should be present
        let hooks = config.get("hooks").unwrap().as_object().unwrap();
        assert_eq!(hooks.len(), 5);
    }
}
