//! Claude Code status tracking setup.
//!
//! Detects Claude Code via the Claude config directory.
//! Installs hooks by merging into Claude Code settings.json.

use anyhow::{Context, Result};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::StatusCheck;

/// Hooks extracted from `.claude-plugin/plugin.json` at compile time.
const PLUGIN_JSON: &str = include_str!("../../../.claude-plugin/plugin.json");

fn claude_dir_from_config(home: PathBuf, config_dir: Option<std::ffi::OsString>) -> PathBuf {
    config_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
}

fn claude_dir() -> Option<PathBuf> {
    home::home_dir().map(|home| claude_dir_from_config(home, std::env::var_os("CLAUDE_CONFIG_DIR")))
}

fn settings_path() -> Option<PathBuf> {
    claude_dir().map(|d| d.join("settings.json"))
}

/// Claude Code's own settings file, honoring `CLAUDE_CONFIG_DIR`.
pub fn settings_file() -> Option<PathBuf> {
    settings_path()
}

/// Where Claude Code keeps declared shell hooks, and its names for the
/// agent-agnostic events. Consumed by `command::setup::agent_hooks`.
pub fn declared_hook_target() -> Option<crate::command::setup::agent_hooks::HookTarget> {
    fn key(event: crate::bootstrap::HookEvent) -> Option<&'static str> {
        use crate::bootstrap::HookEvent;
        match event {
            HookEvent::SessionReady => Some("SessionStart"),
            HookEvent::TurnDone => Some("Stop"),
        }
    }
    Some(crate::command::setup::agent_hooks::HookTarget {
        file: settings_path()?,
        event_key: key,
        requires_plugin: None,
        dialect: crate::command::setup::agent_hooks::HookDialect::Grouped,
    })
}

pub struct Bootstrapper {
    claude_dir: PathBuf,
}

impl Bootstrapper {
    pub fn new() -> Option<Self> {
        claude_dir().map(|d| Self { claude_dir: d })
    }
}

impl super::AgentBootstrapper for Bootstrapper {
    fn instructions_path(&self) -> PathBuf {
        self.claude_dir.join("muxix-bootstrap.md")
    }

    fn apply_prompt(&self, prompt: &str) -> anyhow::Result<()> {
        fs::write(self.instructions_path(), format!("{}\n", prompt.trim()))
            .context("Failed to write ~/.claude/muxix-bootstrap.md")?;

        let claude_md = self.claude_dir.join("CLAUDE.md");
        let existing = if claude_md.exists() {
            fs::read_to_string(&claude_md).context("Failed to read ~/.claude/CLAUDE.md")?
        } else {
            String::new()
        };

        const REF_LINE: &str = "@muxix-bootstrap.md";
        if !existing.lines().any(|l| l.trim() == REF_LINE) {
            let updated = if existing.is_empty() {
                format!("{}\n", REF_LINE)
            } else {
                format!("{}\n{}\n", existing.trim_end(), REF_LINE)
            };
            fs::write(&claude_md, updated).context("Failed to write ~/.claude/CLAUDE.md")?;
        }

        Ok(())
    }
}

/// Detect if Claude Code is present via filesystem.
/// Returns the reason string if detected, None otherwise.
pub fn detect() -> Option<&'static str> {
    if claude_dir().is_some_and(|d| d.is_dir()) {
        return Some("found Claude config directory");
    }

    None
}

/// Check if muxix hooks are installed in Claude Code settings.
///
/// Checks two paths:
/// 1. Plugin: `enabledPlugins` has a key starting with `muxix-status@`
///    (regardless of enabled/disabled -- user knows about it)
/// 2. Manual hooks: `hooks` object contains a command with `muxix set-window-status`
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

/// Check a parsed settings.json value for muxix status tracking configuration.
///
/// Three outcomes, so a *partial* install (e.g. muxix gained new hooks like the
/// pipeline `signal …` set but the user never re-ran setup) is caught instead of
/// silently reading as "installed":
/// - plugin enabled, or every required muxix command present → `Installed`
/// - some muxix commands present but not all → `Stale { missing }`
/// - none → `NotInstalled`
fn check_settings(settings: &Value) -> StatusCheck {
    // Plugin path: if the muxix plugin is present at all (enabled or not — the
    // user knows about it), Claude manages its hooks from plugin.json, so we don't
    // treat it as stale/missing.
    if let Some(plugins) = settings.get("enabledPlugins").and_then(|v| v.as_object())
        && plugins.keys().any(|k| k.starts_with("muxix-status@"))
    {
        return StatusCheck::Installed;
    }

    // Manual-hooks path: compare the installed muxix commands against the full
    // required set extracted from the embedded plugin.json.
    let installed = installed_muxix_commands(settings);
    if installed.is_empty() {
        return StatusCheck::NotInstalled;
    }
    let missing: Vec<String> = required_muxix_commands()
        .into_iter()
        .filter(|cmd| !installed.contains(cmd))
        .collect();
    if missing.is_empty() {
        StatusCheck::Installed
    } else {
        StatusCheck::Stale { missing }
    }
}

/// `true` if the settings contain any muxix hook command at all (regardless of
/// completeness). Kept for tests / callers that only need presence.
#[cfg(test)]
fn has_muxix_hooks(settings: &Value) -> bool {
    !installed_muxix_commands(settings).is_empty()
}

/// `true` if every command in a hook group is a `muxix …` command (i.e. the
/// group is muxix-authored, not a user's own hook sharing the same event).
fn is_muxix_group(group: &Value) -> bool {
    let Some(hooks) = group.get("hooks").and_then(|v| v.as_array()) else {
        return false;
    };
    !hooks.is_empty()
        && hooks.iter().all(|h| {
            h.get("command")
                .and_then(|v| v.as_str())
                .is_some_and(|c| c.trim_start().starts_with("muxix "))
        })
}

/// The full set of `muxix …` hook commands the current build expects, derived
/// from the embedded plugin.json so it stays in sync as hooks are added.
fn required_muxix_commands() -> std::collections::HashSet<String> {
    let plugin: Value =
        serde_json::from_str(PLUGIN_JSON).expect("embedded plugin.json is valid JSON");
    let mut set = std::collections::HashSet::new();
    if let Some(hooks) = plugin.get("hooks").and_then(|v| v.as_object()) {
        collect_muxix_commands(hooks, &mut set);
    }
    set
}

/// The `muxix …` hook commands actually present in a settings.json value.
fn installed_muxix_commands(settings: &Value) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    if let Some(hooks) = settings.get("hooks").and_then(|v| v.as_object()) {
        collect_muxix_commands(hooks, &mut set);
    }
    set
}

/// Walk a `hooks` object (`{event: [{hooks: [{command}]}]}`) collecting every
/// command that starts with `muxix `.
fn collect_muxix_commands(
    hooks: &serde_json::Map<String, Value>,
    out: &mut std::collections::HashSet<String>,
) {
    for groups in hooks.values() {
        let Some(groups_arr) = groups.as_array() else {
            continue;
        };
        for group in groups_arr {
            let Some(hook_list) = group.get("hooks").and_then(|v| v.as_array()) else {
                continue;
            };
            for hook in hook_list {
                if let Some(cmd) = hook.get("command").and_then(|v| v.as_str()) {
                    let cmd = cmd.trim();
                    if cmd.starts_with("muxix ") {
                        out.insert(cmd.to_string());
                    }
                }
            }
        }
    }
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

/// Install muxix hooks into `~/.claude/settings.json`.
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

    // Merge each hook event. To stay idempotent and self-healing on upgrades,
    // first drop any *muxix-authored* groups already present (a stale partial
    // set from an older build), preserving the user's own groups, then insert the
    // current plugin groups. This guarantees exactly the required set with no
    // duplicate / leftover stale groups.
    let hooks_map = hooks_to_add.as_object().expect("plugin hooks is an object");
    for (event, hook_groups) in hooks_map {
        let Some(new_groups) = hook_groups.as_array() else {
            continue;
        };

        if let Some(existing_groups) = existing_hooks.get_mut(event) {
            let arr = existing_groups
                .as_array_mut()
                .ok_or_else(|| anyhow::anyhow!("hooks.{event} is not an array"))?;
            arr.retain(|group| !is_muxix_group(group));
            for group in new_groups {
                arr.push(group.clone());
            }
        } else {
            existing_hooks.insert(event.clone(), hook_groups.clone());
        }
    }

    // Write back with pretty formatting
    let output = serde_json::to_string_pretty(&settings)?;
    fs::write(&path, output + "\n").context("Failed to write ~/.claude/settings.json")?;

    Ok(format!("Installed hooks to {}", path.display()))
}

/// Split a Claude plugin spec into its optional marketplace source and the
/// `plugin@marketplace` id `claude plugin install` expects.
///
/// Claude needs two steps — a marketplace must be registered before a plugin
/// from it can be installed — so the spec carries both, separated by `#`
/// (illegal in both GitHub repo refs and plugin ids, so it never collides):
///
/// - `DietrichGebert/ponytail#ponytail@ponytail` → register the marketplace, then install
/// - `ponytail@ponytail` → install from an already-registered marketplace
fn parse_plugin_spec(spec: &str) -> (Option<&str>, &str) {
    match spec.split_once('#') {
        Some((source, plugin)) => (Some(source.trim()), plugin.trim()),
        None => (None, spec.trim()),
    }
}

/// Whether `spec`'s plugin is already registered in Claude Code's own
/// installed-plugins state, at `scope: "user"` — the scope `install_plugin`
/// always installs at.
///
/// Reads `~/.claude/plugins/installed_plugins.json`
/// (`{"version":2,"plugins":{"<plugin>@<marketplace>":[{"scope":...}]}}`)
/// rather than shelling out to `claude plugin list`, which would cost the
/// process spawn this probe exists to avoid. `spec` is parsed with
/// [`parse_plugin_spec`] to drop any marketplace-source prefix before the `#`,
/// matching only the trailing `plugin@marketplace` id against the state file's
/// keys. On any missing/unparsable state, or the plugin not listed at
/// `scope: "user"`, returns `false` so setup still installs a genuinely
/// missing plugin — a Claude state-format change degrades to today's
/// always-install behavior rather than silently skipping.
pub fn plugin_installed(spec: &str) -> bool {
    let Some(dir) = claude_dir() else {
        return false;
    };
    let Ok(body) = fs::read_to_string(dir.join("plugins/installed_plugins.json")) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<Value>(&body) else {
        return false;
    };
    let (_, plugin_id) = parse_plugin_spec(spec);
    let Some(entries) = json
        .get("plugins")
        .and_then(|p| p.as_object())
        .and_then(|obj| obj.get(plugin_id))
        .and_then(|v| v.as_array())
    else {
        return false;
    };
    entries
        .iter()
        .any(|e| e.get("scope").and_then(|s| s.as_str()) == Some("user"))
}

/// Install a Claude Code plugin using the `claude plugin` CLI.
///
/// Installs at `--scope user` so the plugin loads in every session, including
/// bare launches not managed by muxix — matching how pi/omp plugins register
/// globally in their own settings.
pub fn install_plugin(spec: &str) -> Result<String> {
    let (source, plugin) = parse_plugin_spec(spec);

    if let Some(source) = source {
        // Best-effort: re-adding an already-registered marketplace exits
        // non-zero, and that must not fail setup. A genuinely bad source still
        // surfaces below as a clear "plugin not found" from the install.
        // ponytail: no `marketplace list` pre-check — one process, not two.
        let _ = Command::new("claude")
            .args(["plugin", "marketplace", "add", source, "--scope", "user"])
            .status();
    }

    let status = Command::new("claude")
        .args(["plugin", "install", plugin, "--scope", "user"])
        .status()
        .context("Failed to run `claude plugin install` — is claude on PATH?")?;

    if !status.success() {
        anyhow::bail!("`claude plugin install {}` failed", plugin);
    }

    Ok(format!("Installed Claude Code plugin: {}", plugin))
}

/// Uninstall a Claude Code plugin previously installed by muxix.
///
/// The marketplace registration is deliberately left in place: it is shared by
/// every plugin from that source, and muxix does not track who else needs it.
pub fn uninstall_plugin(spec: &str) -> Result<String> {
    let (_, plugin) = parse_plugin_spec(spec);

    let status = Command::new("claude")
        .args(["plugin", "uninstall", plugin, "--scope", "user"])
        .status()
        .context("Failed to run `claude plugin uninstall` \u{2014} is claude on PATH?")?;

    if !status.success() {
        anyhow::bail!("`claude plugin uninstall {}` failed", plugin);
    }

    Ok(format!("Uninstalled Claude Code plugin: {}", plugin))
}

/// Set the Claude Code theme in `~/.claude/settings.json`.
///
/// Claude Code only ships its own themes (`dark`, `light`, their `-daltonized`
/// / `-ansi` variants, `auto`); the value is passed through as configured, so a
/// theme name Claude does not know is Claude's error to report, not ours.
pub fn set_theme(theme: &str) -> Result<bool> {
    let path =
        settings_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    super::set_json_string(&path, &["theme"], theme)
}

/// Pre-approve the named project MCP servers in `~/.claude/settings.json`, so an
/// agent launched by the harness does not block on the interactive "New MCP
/// server found … Use this MCP server?" trust prompt at startup (which otherwise
/// stalls the pipeline before the first prompt is ever delivered).
///
/// Merges names into `enabledMcpjsonServers` (dedup), preserving the rest of the
/// file. No-op when Claude isn't present (so we don't create config for an agent
/// the user doesn't use) or when there are no servers.
pub fn approve_mcp_servers(names: &[&str]) -> Result<()> {
    if names.is_empty() || detect().is_none() {
        return Ok(());
    }
    let path =
        settings_path().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    let mut settings: Value = if path.exists() {
        let content =
            fs::read_to_string(&path).context("Failed to read ~/.claude/settings.json")?;
        serde_json::from_str(&content).context("~/.claude/settings.json is not valid JSON")?
    } else {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).context("Failed to create ~/.claude/ directory")?;
        }
        Value::Object(serde_json::Map::new())
    };

    let obj = settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("settings.json root is not an object"))?;
    let arr = obj
        .entry("enabledMcpjsonServers".to_string())
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("settings.json enabledMcpjsonServers is not an array"))?;
    for name in names {
        let v = Value::String((*name).to_string());
        if !arr.contains(&v) {
            arr.push(v);
        }
    }

    let output = serde_json::to_string_pretty(&settings)?;
    fs::write(&path, output + "\n").context("Failed to write ~/.claude/settings.json")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // CLAUDE_CONFIG_DIR is process-global; serialize the tests that set it.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_claude_dir(dir: &std::path::Path, f: impl FnOnce()) {
        let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("CLAUDE_CONFIG_DIR");
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir) };
        f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }
    }

    #[test]
    fn plugin_installed_hits_user_scope_entry() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("plugins")).unwrap();
        fs::write(
            tmp.path().join("plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{"ponytail@ponytail":[{"scope":"user"}]}}"#,
        )
        .unwrap();
        with_claude_dir(tmp.path(), || {
            assert!(plugin_installed(
                "DietrichGebert/ponytail#ponytail@ponytail"
            ));
        });
    }

    #[test]
    fn plugin_installed_misses_unlisted_plugin() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("plugins")).unwrap();
        fs::write(
            tmp.path().join("plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{"other@other":[{"scope":"user"}]}}"#,
        )
        .unwrap();
        with_claude_dir(tmp.path(), || {
            assert!(!plugin_installed("ponytail@ponytail"));
        });
    }

    #[test]
    fn plugin_installed_reports_false_on_malformed_state() {
        let tmp = tempfile::tempdir().unwrap();
        with_claude_dir(tmp.path(), || {
            assert!(!plugin_installed("ponytail@ponytail"), "no state file");
        });
        fs::create_dir_all(tmp.path().join("plugins")).unwrap();
        fs::write(
            tmp.path().join("plugins/installed_plugins.json"),
            "not json",
        )
        .unwrap();
        with_claude_dir(tmp.path(), || {
            assert!(!plugin_installed("ponytail@ponytail"), "unparseable state");
        });
    }

    #[test]
    fn plugin_installed_handles_spec_without_hash() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("plugins")).unwrap();
        fs::write(
            tmp.path().join("plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{"ponytail@ponytail":[{"scope":"user"}]}}"#,
        )
        .unwrap();
        with_claude_dir(tmp.path(), || {
            assert!(plugin_installed("ponytail@ponytail"));
        });
    }

    #[test]
    fn test_has_muxix_hooks_empty() {
        let settings = json!({});
        assert!(!has_muxix_hooks(&settings));
    }

    #[test]
    fn test_has_muxix_hooks_present() {
        let settings = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "muxix set-window-status done"
                    }]
                }]
            }
        });
        assert!(has_muxix_hooks(&settings));
    }

    #[test]
    fn test_has_muxix_hooks_other_hooks_only() {
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
        assert!(!has_muxix_hooks(&settings));
    }

    #[test]
    fn test_load_hooks_from_plugin() {
        let hooks = load_hooks_from_plugin().unwrap();
        let obj = hooks.as_object().unwrap();
        assert!(obj.contains_key("UserPromptSubmit"));
        assert!(obj.contains_key("Notification"));
        assert!(obj.contains_key("PostToolUse"));
        assert!(obj.contains_key("Stop"));
        assert!(obj.contains_key("SessionStart"));
    }

    #[test]
    fn test_parse_plugin_spec_with_marketplace_source() {
        let (source, plugin) = parse_plugin_spec("DietrichGebert/ponytail#ponytail@ponytail");
        assert_eq!(source, Some("DietrichGebert/ponytail"));
        assert_eq!(plugin, "ponytail@ponytail");
    }

    #[test]
    fn test_parse_plugin_spec_without_marketplace_source() {
        let (source, plugin) = parse_plugin_spec("ponytail@ponytail");
        assert_eq!(source, None);
        assert_eq!(plugin, "ponytail@ponytail");
    }

    #[test]
    fn test_claude_dir_respects_env() {
        let path = claude_dir_from_config(
            PathBuf::from("/home/test"),
            Some(std::ffi::OsString::from("/tmp/muxix-test-claude-cfg")),
        );
        assert_eq!(path, PathBuf::from("/tmp/muxix-test-claude-cfg"));
    }

    #[test]
    fn test_claude_dir_defaults_to_home() {
        let path = claude_dir_from_config(PathBuf::from("/home/test"), None);
        assert_eq!(path, PathBuf::from("/home/test/.claude"));
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
        assert_eq!(hooks.len(), 5);
    }

    #[test]
    fn test_merge_deduplicates() {
        // Pre-populate with the exact Stop hook group from plugin.json so the
        // contains-check finds a match and does not add a duplicate.
        let mut settings = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [
                        {"type": "command", "command": "muxix set-window-status done"},
                        {"type": "command", "command": "muxix signal turn-done"}
                    ]
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
                "muxix-status@muxix": true
            }
        });
        assert!(matches!(check_settings(&settings), StatusCheck::Installed));
    }

    #[test]
    fn test_check_settings_plugin_disabled() {
        let settings = json!({
            "enabledPlugins": {
                "muxix-status@muxix": false
            }
        });
        assert!(matches!(check_settings(&settings), StatusCheck::Installed));
    }

    #[test]
    fn test_check_settings_plugin_different_version() {
        let settings = json!({
            "enabledPlugins": {
                "muxix-status@1.2.3": true
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
    fn test_check_settings_hooks_partial_is_stale() {
        // Only the status hook present (the old install) — the pipeline `signal …`
        // hooks are missing, so this must read as Stale, not Installed.
        let settings = json!({
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "muxix set-window-status done"
                    }]
                }]
            }
        });
        match check_settings(&settings) {
            StatusCheck::Stale { missing } => {
                assert!(missing.iter().any(|m| m == "muxix signal turn-done"));
            }
            other => panic!("expected Stale, got {other:?}"),
        }
    }

    #[test]
    fn test_check_settings_full_hooks_installed() {
        // The complete plugin hook set → Installed.
        let plugin: Value = serde_json::from_str(PLUGIN_JSON).unwrap();
        let settings = json!({ "hooks": plugin.get("hooks").unwrap() });
        assert!(matches!(check_settings(&settings), StatusCheck::Installed));
    }

    #[test]
    fn test_check_settings_both_plugin_and_hooks() {
        let settings = json!({
            "enabledPlugins": {
                "muxix-status@muxix": true
            },
            "hooks": {
                "Stop": [{
                    "hooks": [{
                        "type": "command",
                        "command": "muxix set-window-status done"
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

        // Stop should have 2 groups (original afplay + muxix)
        let stop = settings
            .get("hooks")
            .unwrap()
            .get("Stop")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(stop.len(), 2);

        // All 5 events should be present (incl. SessionStart for MCP readiness).
        let hooks = settings.get("hooks").unwrap().as_object().unwrap();
        assert_eq!(hooks.len(), 5);
    }
}
