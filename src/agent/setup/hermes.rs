//! Hermes Agent status tracking setup.
//!
//! Detects Hermes Agent via the `~/.hermes/` directory, overridable with
//! `HERMES_HOME`. Installs hooks by merging into `~/.hermes/config.yaml`
//! (YAML, unlike every other agent's JSON settings file).

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use super::StatusCheck;

/// Hook block embedded at compile time, as JSON. hermes' own config store is
/// YAML, but the embedded block is plain JSON (easier to author and diff) and
/// gets merged into the parsed YAML document before being written back.
const HOOKS_JSON: &str = include_str!("../../../resources/hermes/hooks.json");

/// hermes' home directory, honoring `HERMES_HOME`.
pub fn agent_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("HERMES_HOME") {
        return Some(PathBuf::from(dir));
    }
    home::home_dir().map(|h| h.join(".hermes"))
}

fn settings_path() -> Option<PathBuf> {
    agent_dir().map(|d| d.join("config.yaml"))
}

/// hermes' own settings file.
pub fn settings_file() -> Option<PathBuf> {
    settings_path()
}

/// hermes' global prompt file: the only global instruction file it reads
/// (`AGENTS.md` / `.hermes.md` are project-local). hermes seeds this itself
/// and never overwrites it, so the default sentinel injection is safe.
pub struct Bootstrapper {
    agent_dir: PathBuf,
}

impl Bootstrapper {
    pub fn new() -> Option<Self> {
        agent_dir().map(|d| Self { agent_dir: d })
    }
}

impl super::AgentBootstrapper for Bootstrapper {
    fn instructions_path(&self) -> PathBuf {
        self.agent_dir.join("SOUL.md")
    }
}

/// Detect if Hermes Agent is present via filesystem.
pub fn detect() -> Option<&'static str> {
    if std::env::var("HERMES_HOME").is_ok_and(|d| PathBuf::from(d).is_dir()) {
        return Some("found $HERMES_HOME");
    }
    if agent_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.hermes/");
    }
    None
}

/// The embedded hook block, parsed.
fn load_block() -> Value {
    serde_json::from_str(HOOKS_JSON).expect("embedded hermes hooks.json is valid JSON")
}

/// Whether `config` already carries every muxix hook command for every
/// declared event, AND `hooks_auto_accept: true`. Both halves matter: a hook
/// present without consent never fires, so it does not count as installed.
fn has_muxix_hooks(config: &Value) -> bool {
    let block = load_block();
    let want_hooks = block
        .get("hooks")
        .and_then(|v| v.as_object())
        .expect("embedded block has a hooks object");
    let Some(have_hooks) = config.get("hooks").and_then(|v| v.as_object()) else {
        return false;
    };

    for (event, want_entries) in want_hooks {
        let Some(want_arr) = want_entries.as_array() else {
            continue;
        };
        let Some(have_arr) = have_hooks.get(event).and_then(|v| v.as_array()) else {
            return false;
        };
        for want_entry in want_arr {
            let Some(want_cmd) = want_entry.get("command").and_then(|v| v.as_str()) else {
                continue;
            };
            let found = have_arr
                .iter()
                .any(|e| e.get("command").and_then(|v| v.as_str()) == Some(want_cmd));
            if !found {
                return false;
            }
        }
    }

    config.get("hooks_auto_accept").and_then(|v| v.as_bool()) == Some(true)
}

/// Check if muxix hooks and `hooks_auto_accept` are installed in
/// `~/.hermes/config.yaml`.
pub fn check() -> Result<StatusCheck> {
    let Some(dir) = agent_dir() else {
        return Ok(StatusCheck::NotInstalled);
    };
    check_in(&dir)
}

/// `check()` against an explicit hermes home, so tests never mutate `HERMES_HOME`.
fn check_in(dir: &std::path::Path) -> Result<StatusCheck> {
    let path = dir.join("config.yaml");
    if !path.exists() {
        return Ok(StatusCheck::NotInstalled);
    }

    let content = fs::read_to_string(&path).context("Failed to read ~/.hermes/config.yaml")?;
    let config: Value =
        serde_yaml::from_str(&content).context("~/.hermes/config.yaml is not valid YAML")?;

    if has_muxix_hooks(&config) {
        Ok(StatusCheck::Installed)
    } else {
        Ok(StatusCheck::NotInstalled)
    }
}

/// Install muxix hooks and `hooks_auto_accept: true` into
/// `~/.hermes/config.yaml`, merging into whatever is already there.
///
/// `hooks_auto_accept: true` auto-approves EVERY shell hook in that file, not
/// just the ones muxix wrote — hermes gates all of them behind the same
/// first-use consent flag, so there is no narrower switch. Callers that
/// surface this to a report should say so explicitly.
pub fn install() -> Result<String> {
    let dir = agent_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    install_in(&dir)
}

/// `install()` against an explicit hermes home, so tests never mutate `HERMES_HOME`.
fn install_in(dir: &std::path::Path) -> Result<String> {
    let path = dir.join("config.yaml");

    let mut config: Value = if path.exists() {
        let content = fs::read_to_string(&path).context("Failed to read ~/.hermes/config.yaml")?;
        serde_yaml::from_str(&content).context("~/.hermes/config.yaml is not valid YAML")?
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create ~/.hermes/ directory")?;
        }
        Value::Object(serde_json::Map::new())
    };

    let config_obj = config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("~/.hermes/config.yaml root is not a mapping"))?;

    if !config_obj.contains_key("hooks") {
        config_obj.insert("hooks".to_string(), Value::Object(serde_json::Map::new()));
    }
    let existing_hooks = config_obj
        .get_mut("hooks")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("~/.hermes/config.yaml `hooks` is not a mapping"))?;

    let block = load_block();
    let want_hooks = block
        .get("hooks")
        .and_then(|v| v.as_object())
        .expect("embedded block has a hooks object");

    for (event, want_entries) in want_hooks {
        let Some(want_arr) = want_entries.as_array() else {
            continue;
        };
        if let Some(existing) = existing_hooks.get_mut(event) {
            let arr = existing.as_array_mut().ok_or_else(|| {
                anyhow::anyhow!("~/.hermes/config.yaml hooks.{event} is not a list")
            })?;
            for entry in want_arr {
                if !arr.contains(entry) {
                    arr.push(entry.clone());
                }
            }
        } else {
            existing_hooks.insert(event.clone(), want_entries.clone());
        }
    }

    config_obj.insert("hooks_auto_accept".to_string(), Value::Bool(true));

    // A store symlink (Home Manager / NixOS) is the common read-only case, and
    // hermes' config is a plausible thing to declare there; the write would
    // otherwise fail with a bare EACCES.
    if fs::metadata(&path).is_ok_and(|m| m.permissions().readonly()) {
        anyhow::bail!(
            "{} is read-only (managed by Nix/Home Manager?) — declare the muxix `hooks:` block              and `hooks_auto_accept: true` in its source and rebuild",
            path.display()
        );
    }

    let output = serde_yaml::to_string(&config)?;
    fs::write(&path, output).context("Failed to write ~/.hermes/config.yaml")?;

    Ok(
        "Installed hooks to ~/.hermes/config.yaml and set hooks_auto_accept: true \
         (this auto-approves every shell hook in that file, including any muxix did not write)"
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hooks_json_is_valid_and_uses_plugin_hook_names() {
        let block = load_block();
        let hooks = block.get("hooks").unwrap().as_object().unwrap();
        // Plugin-hook names hermes accepts for shell hooks, never gateway
        // names (agent:start/agent:end/agent:step), which hermes drops.
        for event in [
            "pre_llm_call",
            "post_tool_call",
            "pre_approval_request",
            "on_session_end",
        ] {
            assert!(hooks.contains_key(event), "missing {event}");
        }
        assert_eq!(block.get("hooks_auto_accept"), Some(&json!(true)));
    }

    #[test]
    fn has_muxix_hooks_false_when_absent() {
        assert!(!has_muxix_hooks(&json!({})));
    }

    #[test]
    fn has_muxix_hooks_false_without_auto_accept() {
        // All hook entries present, but hooks_auto_accept missing: not installed,
        // because an unapproved hook never fires.
        let config = json!({
            "hooks": {
                "pre_llm_call": [{"command": "muxix set-window-status working"}],
                "post_tool_call": [{"command": "muxix set-window-status working"}],
                "pre_approval_request": [{"command": "muxix set-window-status waiting"}],
                "on_session_end": [{"command": "muxix set-window-status done"}]
            }
        });
        assert!(!has_muxix_hooks(&config));
    }

    #[test]
    fn has_muxix_hooks_true_when_complete() {
        let mut config = load_block();
        assert!(has_muxix_hooks(&config));

        // Extra unrelated keys and hooks must not break detection.
        config.as_object_mut().unwrap().insert(
            "mcp_servers".to_string(),
            json!({"foo": {"command": "bar"}}),
        );
        assert!(has_muxix_hooks(&config));
    }

    #[test]
    fn install_merges_into_existing_config_preserving_unrelated_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.yaml");
        fs::write(
            &path,
            "model: some-model\nmcp_servers:\n  foo:\n    command: bar\n",
        )
        .unwrap();

        assert!(matches!(
            check_in(tmp.path()).unwrap(),
            StatusCheck::NotInstalled
        ));

        let msg = install_in(tmp.path()).unwrap();
        assert!(msg.contains("hooks_auto_accept"));

        let parsed: Value = serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed.get("model").and_then(|v| v.as_str()),
            Some("some-model")
        );
        assert!(parsed.get("mcp_servers").unwrap().get("foo").is_some());
        assert!(has_muxix_hooks(&parsed));
        assert!(matches!(
            check_in(tmp.path()).unwrap(),
            StatusCheck::Installed
        ));
    }

    #[test]
    fn install_refuses_a_read_only_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.yaml");
        fs::write(&path, "model: some-model\n").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms).unwrap();

        let err = install_in(tmp.path()).unwrap_err().to_string();
        assert!(err.contains("read-only"), "{err}");
    }

    #[test]
    fn install_is_idempotent_and_seeds_a_missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("fresh");

        install_in(&home).unwrap();
        let first = fs::read_to_string(home.join("config.yaml")).unwrap();
        install_in(&home).unwrap();
        assert_eq!(first, fs::read_to_string(home.join("config.yaml")).unwrap());
    }
}
