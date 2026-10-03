//! Codex status tracking setup.
//!
//! Detects Codex via the `~/.codex/` directory.
//! Installs hooks by merging into `~/.codex/hooks.json`.
//!
//! Codex hooks require enabling the feature flag in `~/.codex/config.toml`:
//! ```toml
//! [features]
//! hooks = true
//! ```

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use super::StatusCheck;

/// Hooks configuration embedded at compile time.
const HOOKS_JSON: &str = include_str!("../../../.codex/hooks/muxix-status.json");

/// Codex's home directory, honoring `CODEX_HOME` (the var a muxix agent
/// profile redirects).
pub fn codex_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CODEX_HOME") {
        return Some(PathBuf::from(dir));
    }
    home::home_dir().map(|h| h.join(".codex"))
}

/// Codex's global instructions file: the first link in its instruction chain
/// (`$CODEX_HOME/AGENTS.md`), read before any repository `AGENTS.md`.
///
/// `AGENTS.override.md` sits above it and is deliberately left to the user:
/// muxix writing there would silence the user's own repo instructions.
pub fn instructions_file() -> Option<PathBuf> {
    codex_dir().map(|d| d.join("AGENTS.md"))
}

/// Codex global instructions bootstrapper: a muxix-managed sentinel region in
/// `$CODEX_HOME/AGENTS.md`, so hand-written guidance around it survives.
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

fn hooks_path() -> Option<PathBuf> {
    codex_dir().map(|d| d.join("hooks.json"))
}

/// Detect if Codex is present via filesystem.
pub fn detect() -> Option<&'static str> {
    if codex_dir().is_some_and(|d| d.is_dir()) {
        return Some("found ~/.codex/");
    }
    None
}

/// Check if muxix hooks are installed in Codex hooks.json.
pub fn check() -> Result<StatusCheck> {
    let Some(path) = hooks_path() else {
        return Ok(StatusCheck::NotInstalled);
    };

    if !path.exists() {
        return Ok(StatusCheck::NotInstalled);
    }

    let content = fs::read_to_string(&path).context("Failed to read ~/.codex/hooks.json")?;
    let config: Value =
        serde_json::from_str(&content).context("~/.codex/hooks.json is not valid JSON")?;

    if has_muxix_hooks(&config) {
        Ok(StatusCheck::Installed)
    } else {
        Ok(StatusCheck::NotInstalled)
    }
}

/// Check if the hooks object contains any muxix set-window-status commands.
fn has_muxix_hooks(config: &Value) -> bool {
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
                    && cmd.contains("muxix set-window-status")
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

fn config_toml_path() -> Option<PathBuf> {
    codex_dir().map(|d| d.join("config.toml"))
}

const PROVIDERS_BEGIN: &str = "# muxix:providers begin";
const PROVIDERS_END: &str = "# muxix:providers end";

/// Render the marker-delimited `[model_providers.*]` region for all syncable
/// registry providers. Also returns skip warnings. Codex has no env-var
/// substitution in config values, so a `base_url` embedding `{env:...}` cannot
/// be expressed — those providers are skipped rather than written broken.
/// Secrets are fine: Codex's `env_key` natively takes the variable *name*.
fn render_providers_region(
    registry: &crate::model::ProviderRegistry,
) -> (String, Vec<String>) {
    let mut region = String::new();
    let mut warnings = Vec::new();
    for (id, cfg) in registry {
        if !cfg.has_connection() {
            continue;
        }
        if cfg.api() == crate::model::ProviderApi::Anthropic {
            warnings.push(format!(
                "Skipped provider `{id}` for Codex: wire_api supports only the OpenAI protocol"
            ));
            continue;
        }
        let Some(base) = &cfg.base_url else {
            warnings.push(format!(
                "Skipped provider `{id}` for Codex: no base_url declared"
            ));
            continue;
        };
        if base.contains("{env:") {
            warnings.push(format!(
                "Skipped provider `{id}` for Codex: base_url `{base}` uses an env reference, which Codex config does not support"
            ));
            continue;
        }
        region.push_str(&format!(
            "[model_providers.{id}]\nname = \"{id}\"\nbase_url = \"{base}\"\nwire_api = \"chat\"\n"
        ));
        if let Some(var) = &cfg.api_key_env {
            region.push_str(&format!("env_key = \"{var}\"\n"));
        }
    }
    (region, warnings)
}

/// Splice the managed region into an existing config.toml body. Content
/// outside the markers is preserved byte-for-byte. Returns None when the file
/// already contains exactly this region (or has no region and none is needed).
fn splice_providers_region(content: &str, region: &str) -> Option<String> {
    splice_region(content, PROVIDERS_BEGIN, PROVIDERS_END, region)
}

/// Marker-delimited managed-region splice over a TOML body.
///
/// Shared by every managed region muxix owns in `config.toml` (providers, MCP
/// servers): the markers are the only thing that differs, and content outside
/// them is never touched.
pub fn splice_region(
    content: &str,
    begin: &str,
    end_marker: &str,
    region: &str,
) -> Option<String> {
    let block = if region.is_empty() {
        String::new()
    } else {
        format!("{begin}\n{region}{end_marker}\n")
    };

    match (content.find(begin), content.find(end_marker)) {
        (Some(start), Some(end)) => {
            let end = end + end_marker.len();
            // Swallow the trailing newline of the old block so an emptied
            // region doesn't leave a blank line behind.
            let end = if content[end..].starts_with('\n') { end + 1 } else { end };
            let current = &content[start..end];
            if current == block {
                return None;
            }
            Some(format!("{}{}{}", &content[..start], block, &content[end..]))
        }
        _ => {
            if block.is_empty() {
                return None;
            }
            let sep = if content.is_empty() || content.ends_with('\n') { "" } else { "\n" };
            Some(format!("{content}{sep}{block}"))
        }
    }
}

const MCP_BEGIN: &str = "# muxix:mcp begin";
const MCP_END: &str = "# muxix:mcp end";

/// TOML string literal: basic form with the few escapes TOML requires.
fn toml_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

/// Render the managed `[mcp_servers.*]` region for `servers`.
///
/// Codex names the table `mcp_servers` and takes `command`, `args` and `env`
/// in the same shape every other agent uses, so the declaration maps across
/// without interpretation.
pub fn render_mcp_region(
    servers: &std::collections::BTreeMap<String, crate::config::McpServerConfig>,
) -> String {
    let mut region = String::new();
    for (name, cfg) in servers {
        region.push_str(&format!("[mcp_servers.{name}]\n"));
        region.push_str(&format!("command = {}\n", toml_string(&cfg.command)));
        if let Some(args) = &cfg.args {
            let rendered: Vec<String> = args.iter().map(|a| toml_string(a)).collect();
            region.push_str(&format!("args = [{}]\n", rendered.join(", ")));
        }
        if let Some(env) = cfg.env.as_ref().filter(|e| !e.is_empty()) {
            region.push_str(&format!("[mcp_servers.{name}.env]\n"));
            for (k, v) in env {
                region.push_str(&format!("{k} = {}\n", toml_string(v)));
            }
        }
    }
    region
}

/// Splice the declared MCP servers into a `config.toml` body (project layer or
/// global). Hand-written entries outside the markers are preserved.
pub fn splice_mcp_region(content: &str, region: &str) -> Option<String> {
    splice_region(content, MCP_BEGIN, MCP_END, region)
}

/// Mark `project_root` trusted in the GLOBAL `~/.codex/config.toml`.
///
/// Codex loads a project's `.codex/config.toml` layer only for a trusted
/// project, so without this the MCP config muxix just wrote is ignored.
pub fn trust_project(project_root: &std::path::Path, dry_run: bool) -> Result<bool> {
    let path =
        config_toml_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let content = if path.exists() {
        fs::read_to_string(&path).context("Failed to read ~/.codex/config.toml")?
    } else {
        String::new()
    };

    let key = format!(
        "[projects.{}]",
        toml_string(&project_root.to_string_lossy())
    );
    let entry = format!("{key}\ntrust_level = \"trusted\"\n");
    if content.contains(&entry) {
        return Ok(false);
    }
    if content.contains(&key) {
        // The project is already declared with some other trust level: that is
        // the user's call, not ours to overwrite.
        return Ok(false);
    }
    if dry_run {
        return Ok(true);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create ~/.codex/ directory")?;
    }
    let sep = if content.is_empty() || content.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    fs::write(&path, format!("{content}{sep}{entry}"))
        .context("Failed to write ~/.codex/config.toml")?;
    Ok(true)
}

/// Sync registry providers with connection details into `~/.codex/config.toml`
/// via a marker-delimited managed region. Returns human-readable result lines
/// (empty = nothing to do).
pub fn sync_providers(
    registry: &crate::model::ProviderRegistry,
    dry_run: bool,
) -> Result<Vec<String>> {
    if !registry.values().any(|c| c.has_connection()) {
        return Ok(Vec::new());
    }
    let path =
        config_toml_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    let content = if path.exists() {
        fs::read_to_string(&path).context("Failed to read ~/.codex/config.toml")?
    } else {
        String::new()
    };

    let (region, mut messages) = render_providers_region(registry);
    let Some(updated) = splice_providers_region(&content, &region) else {
        return Ok(messages);
    };

    if dry_run {
        messages.push(format!("Would update provider region in {}", path.display()));
        return Ok(messages);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create ~/.codex/ directory")?;
    }
    fs::write(&path, &updated).context("Failed to write ~/.codex/config.toml")?;
    messages.push(format!("Updated provider region in {}", path.display()));
    Ok(messages)
}


/// Ensure `hooks = true` is set under `[features]` in config.toml.
/// Returns true if the file was modified.
fn ensure_hooks_feature_flag() -> Result<bool> {
    let path =
        config_toml_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    let content = if path.exists() {
        fs::read_to_string(&path).context("Failed to read ~/.codex/config.toml")?
    } else {
        String::new()
    };

    // Check if already enabled
    if is_hooks_feature_enabled(&content) {
        return Ok(false);
    }

    let updated = if has_hooks_feature_key(&content) {
        // Replace existing hooks value
        content
            .lines()
            .map(|line| {
                if line
                    .trim()
                    .split_once('=')
                    .is_some_and(|(key, _)| key.trim() == "hooks")
                {
                    "hooks = true"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + if content.ends_with('\n') { "\n" } else { "" }
    } else if content.contains("[features]") {
        // Insert after the [features] line
        content.replacen("[features]", "[features]\nhooks = true", 1)
    } else {
        // Append a new [features] section
        let sep = if content.is_empty() || content.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        format!("{content}{sep}\n[features]\nhooks = true\n")
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create ~/.codex/ directory")?;
    }
    fs::write(&path, &updated).context("Failed to write ~/.codex/config.toml")?;

    Ok(true)
}


/// Codex keeps shell hooks in `~/.codex/hooks.json`, same inner shape as
/// Claude's settings. It has a `Stop` event but nothing that fires once per
/// session, so `session-ready` is not expressible.
pub fn declared_hook_target() -> Option<crate::command::setup::agent_hooks::HookTarget> {
    fn key(event: crate::bootstrap::HookEvent) -> Option<&'static str> {
        use crate::bootstrap::HookEvent;
        match event {
            HookEvent::SessionReady => None,
            HookEvent::TurnDone => Some("Stop"),
        }
    }
    Some(crate::command::setup::agent_hooks::HookTarget {
        file: hooks_path()?,
        event_key: key,
        requires_plugin: None,
        dialect: crate::command::setup::agent_hooks::HookDialect::Grouped,
    })
}

/// Check if `hooks = true` is set in the config content.
fn is_hooks_feature_enabled(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "hooks = true" || trimmed == "hooks=true"
    })
}

/// Check if `hooks` key exists at all (regardless of value).
fn has_hooks_feature_key(content: &str) -> bool {
    content.lines().any(|line| {
        line.trim()
            .split_once('=')
            .is_some_and(|(key, _)| key.trim() == "hooks")
    })
}

/// Install muxix hooks into `~/.codex/hooks.json`.
///
/// Merges hook groups into existing hooks without clobbering or creating
/// duplicates. Returns a description of what was done.
pub fn install() -> Result<String> {
    let path = hooks_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    // Read existing config or start fresh
    let mut config: Value = if path.exists() {
        let content = fs::read_to_string(&path).context("Failed to read ~/.codex/hooks.json")?;
        serde_json::from_str(&content).context("~/.codex/hooks.json is not valid JSON")?
    } else {
        // Ensure ~/.codex/ directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create ~/.codex/ directory")?;
        }
        serde_json::json!({ "hooks": {} })
    };

    let hooks_to_add = load_hooks()?;

    // Ensure config.hooks exists as an object
    let config_obj = config
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("hooks.json root is not an object"))?;

    if !config_obj.contains_key("hooks") {
        config_obj.insert("hooks".to_string(), Value::Object(serde_json::Map::new()));
    }

    let existing_hooks = config_obj
        .get_mut("hooks")
        .and_then(|v| v.as_object_mut())
        .ok_or_else(|| anyhow::anyhow!("hooks.json hooks is not an object"))?;

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
    fs::write(&path, output + "\n").context("Failed to write ~/.codex/hooks.json")?;

    // Ensure hooks feature flag is enabled in config.toml
    let feature_msg = match ensure_hooks_feature_flag() {
        Ok(true) => ", enabled hooks in ~/.codex/config.toml",
        _ => "",
    };

    Ok(format!(
        "Installed hooks to ~/.codex/hooks.json{feature_msg}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use super::super::AgentBootstrapper;

    #[test]
    fn global_instructions_region_is_managed_and_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("AGENTS.md");
        std::fs::write(&path, "# hand-written\n\nkeep me\n").unwrap();
        let b = Bootstrapper {
            instructions: path.clone(),
        };

        b.apply_prompt("declared component").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("keep me"), "{body}");
        assert!(body.contains("declared component"), "{body}");
        assert_eq!(b.current_prompt().as_deref(), Some("declared component"));

        // Second apply of the same prompt leaves one region, not two.
        b.apply_prompt("declared component").unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body.matches("muxix-bootstrap-begin").count(), 1, "{body}");
    }

    #[test]
    fn providers_region_create_rewrite_and_preserve() {
        use crate::model::{ProviderConfig, ProviderRegistry};
        let mut reg = ProviderRegistry::new();
        reg.insert(
            "litellm".into(),
            ProviderConfig {
                base_url: Some("https://llm.corp/v1".into()),
                api_key_env: Some("LITELLM_API_KEY".into()),
                ..Default::default()
            },
        );

        let existing = "[features]\nhooks = true\n";
        let (region, warnings) = render_providers_region(&reg);
        assert!(warnings.is_empty());
        let updated = splice_providers_region(existing, &region).unwrap();
        assert!(updated.starts_with(existing));
        assert!(updated.contains("[model_providers.litellm]"));
        assert!(updated.contains("base_url = \"https://llm.corp/v1\""));
        assert!(updated.contains("env_key = \"LITELLM_API_KEY\""));

        // Idempotent re-splice.
        assert!(splice_providers_region(&updated, &region).is_none());

        // Rewrite touches only the region.
        reg.get_mut("litellm").unwrap().base_url = Some("https://other/v1".into());
        let (region2, _) = render_providers_region(&reg);
        let rewritten = splice_providers_region(&updated, &region2).unwrap();
        assert!(rewritten.starts_with(existing));
        assert!(rewritten.contains("https://other/v1"));
        assert!(!rewritten.contains("llm.corp"));
    }

    #[test]
    fn providers_region_skips_env_ref_base_url() {
        use crate::model::{ProviderConfig, ProviderRegistry};
        let mut reg = ProviderRegistry::new();
        reg.insert(
            "litellm".into(),
            ProviderConfig {
                base_url: Some("{env:LITELLM_BASE_URL}".into()),
                api_key_env: Some("LITELLM_API_KEY".into()),
                ..Default::default()
            },
        );
        let (region, warnings) = render_providers_region(&reg);
        assert!(region.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("litellm"));
        assert!(warnings[0].contains("env reference"));
        // No region needed + none present -> untouched.
        assert!(splice_providers_region("x = 1\n", &region).is_none());
    }

    #[test]
    fn test_hooks_json_is_valid() {
        let parsed: serde_json::Value =
            serde_json::from_str(HOOKS_JSON).expect("embedded hooks config is valid JSON");
        let hooks = parsed.get("hooks").unwrap().as_object().unwrap();
        assert!(hooks.contains_key("UserPromptSubmit"));
        assert!(hooks.contains_key("PostToolUse"));
        assert!(hooks.contains_key("Stop"));
    }

    #[test]
    fn test_hooks_json_contains_muxix_command() {
        assert!(HOOKS_JSON.contains("muxix set-window-status"));
    }

    #[test]
    fn test_has_muxix_hooks_empty() {
        let config = json!({});
        assert!(!has_muxix_hooks(&config));
    }

    #[test]
    fn test_has_muxix_hooks_present() {
        let config = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "muxix set-window-status done"
                    }]
                }]
            }
        });
        assert!(has_muxix_hooks(&config));
    }

    #[test]
    fn test_has_muxix_hooks_other_hooks_only() {
        let config = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "python3 my-script.py"
                    }]
                }]
            }
        });
        assert!(!has_muxix_hooks(&config));
    }

    #[test]
    fn test_load_hooks() {
        let hooks = load_hooks().unwrap();
        let obj = hooks.as_object().unwrap();
        assert!(obj.contains_key("UserPromptSubmit"));
        assert!(obj.contains_key("PostToolUse"));
        assert!(obj.contains_key("Stop"));
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
        assert_eq!(hooks.len(), 3);
    }

    #[test]
    fn test_merge_deduplicates() {
        let mut config = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "muxix set-window-status done"
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

        // Stop should still have exactly 1 group (not duplicated)
        let stop = config
            .get("hooks")
            .unwrap()
            .get("Stop")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(stop.len(), 1);
    }

    #[test]
    fn test_merge_preserves_existing_hooks() {
        let mut config = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "python3 my-stop-hook.py"
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

        // Stop should have 2 groups (original + muxix)
        let stop = config
            .get("hooks")
            .unwrap()
            .get("Stop")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(stop.len(), 2);

        // All 3 events should be present
        let hooks = config.get("hooks").unwrap().as_object().unwrap();
        assert_eq!(hooks.len(), 3);
    }

    #[test]
    fn test_is_hooks_feature_enabled_true() {
        assert!(is_hooks_feature_enabled("[features]\nhooks = true\n"));
    }

    #[test]
    fn test_is_hooks_feature_enabled_no_spaces() {
        assert!(is_hooks_feature_enabled("[features]\nhooks=true\n"));
    }

    #[test]
    fn test_is_hooks_feature_enabled_with_other_settings() {
        let content = "[model]\ndefault = \"gpt-4\"\n\n[features]\nhooks = true\n";
        assert!(is_hooks_feature_enabled(content));
    }

    #[test]
    fn test_is_hooks_feature_enabled_false() {
        assert!(!is_hooks_feature_enabled("[features]\nhooks = false\n"));
    }

    #[test]
    fn test_has_hooks_feature_key_true() {
        assert!(has_hooks_feature_key("[features]\nhooks = true\n"));
    }

    #[test]
    fn test_has_hooks_feature_key_false() {
        assert!(has_hooks_feature_key("[features]\nhooks = false\n"));
    }

    #[test]
    fn test_has_hooks_feature_key_missing() {
        assert!(!has_hooks_feature_key("[features]\n"));
    }

    #[test]
    fn test_is_hooks_feature_enabled_empty() {
        assert!(!is_hooks_feature_enabled(""));
    }

    #[test]
    fn test_is_hooks_feature_enabled_no_features_section() {
        assert!(!is_hooks_feature_enabled("[model]\ndefault = \"gpt-4\"\n"));
    }
}
