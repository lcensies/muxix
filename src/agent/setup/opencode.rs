//! OpenCode status tracking setup.
//!
//! Detects OpenCode via its config directory. Resolution order:
//! 1. `OPENCODE_CONFIG` env var (explicit override)
//! 2. `XDG_CONFIG_HOME/opencode`
//! 3. `~/.config/opencode`
//!
//! Installs plugin by writing `package.json` and `muxix-status.ts` to the
//! OpenCode config directory.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::StatusCheck;

/// OpenCode distribution files, embedded at compile time.
const PLUGIN_SOURCE: &str = include_str!("../../../resources/opencode/plugins/muxix-status.ts");
const PACKAGE_JSON: &str = include_str!("../../../resources/opencode/package.json");

pub fn opencode_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("OPENCODE_CONFIG") {
        return Some(PathBuf::from(dir));
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return Some(PathBuf::from(xdg).join("opencode"));
    }
    home::home_dir().map(|h| h.join(".config/opencode"))
}

/// OpenCode's own config file. OpenCode merges `config.json`, `opencode.json`
/// and `opencode.jsonc`; muxix writes the middle one, same as theme and
/// provider sync.
pub fn settings_file() -> Option<PathBuf> {
    opencode_config_dir().map(|d| d.join("opencode.json"))
}

fn plugin_path() -> Option<PathBuf> {
    opencode_config_dir().map(|d| d.join("plugins/muxix-status.ts"))
}

/// Where OpenCode's `opencode-claude-hooks` compat plugin reads Claude-format
/// hooks. The plugin also reads `.claude/settings*.json` and concatenates all
/// files per event, so a hook installed for Claude Code may fire here too —
/// hook scripts must tolerate running twice per event.
pub fn declared_hook_target() -> Option<crate::command::setup::agent_hooks::HookTarget> {
    use crate::command::setup::agent_hooks::{HookDialect, HookTarget, RequiredPlugin};
    fn key(event: crate::bootstrap::HookEvent) -> Option<&'static str> {
        use crate::bootstrap::HookEvent;
        match event {
            HookEvent::SessionReady => Some("SessionStart"),
            HookEvent::TurnDone => Some("Stop"),
        }
    }
    Some(HookTarget {
        file: opencode_config_dir()?.join("hooks.json"),
        event_key: key,
        requires_plugin: Some(RequiredPlugin {
            spec: "opencode-claude-hooks",
            name_fragment: "opencode-claude-hooks",
        }),
        dialect: HookDialect::Grouped,
    })
}

fn legacy_plugin_path() -> Option<PathBuf> {
    opencode_config_dir().map(|d| d.join("plugin/muxix-status.ts"))
}

fn package_json_path() -> Option<PathBuf> {
    opencode_config_dir().map(|d| d.join("package.json"))
}

pub struct Bootstrapper {
    config_dir: PathBuf,
}

impl Bootstrapper {
    pub fn new() -> Option<Self> {
        opencode_config_dir().map(|d| Self { config_dir: d })
    }
}

impl super::AgentBootstrapper for Bootstrapper {
    fn instructions_path(&self) -> PathBuf {
        self.config_dir.join("AGENTS.md")
    }

    fn apply_prompt(&self, prompt: &str) -> anyhow::Result<()> {
        fs::create_dir_all(&self.config_dir)
            .context("Failed to create OpenCode config directory")?;
        super::inline_sentinels(&self.instructions_path(), prompt)
    }
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

/// Check if muxix plugin is installed for OpenCode.
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

/// Whether `module` is already registered in OpenCode's global `opencode.json`
/// `plugin` array. Exact membership only, so a miss (e.g. a version-suffixed
/// entry) safely falls back to reinstalling rather than skipping.
pub fn plugin_installed(module: &str) -> bool {
    let Some(dir) = opencode_config_dir() else {
        return false;
    };
    let Ok(body) = std::fs::read_to_string(dir.join("opencode.json")) else {
        return false;
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else {
        return false;
    };
    json.get("plugin")
        .and_then(|p| p.as_array())
        .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some(module)))
}

/// Install an OpenCode plugin from an npm module spec using `opencode plugin`.
///
/// This is the correct mechanism — `opencode plugin <module> --global` installs
/// the module *and* registers it in the global config, so it loads on every
/// OpenCode start, including bare launches not managed by muxix. `--force`
/// keeps re-runs idempotent by replacing an already-installed version instead of
/// failing.
pub fn install_plugin(module: &str) -> Result<String> {
    let status = Command::new("opencode")
        .args(["plugin", module, "--global", "--force"])
        .status()
        .context("Failed to run `opencode plugin` — is opencode on PATH?")?;

    if !status.success() {
        anyhow::bail!("`opencode plugin {} --global` failed", module);
    }

    Ok(format!("Installed OpenCode plugin: {}", module))
}

/// Deregister an OpenCode plugin from the global `opencode.json`.
///
/// `opencode` has an install subcommand but no uninstall one, so removal is a
/// config edit: drop the module from the `plugin` array. The downloaded module
/// stays in OpenCode's own cache \u2014 unreferenced, so it no longer loads.
pub fn uninstall_plugin(module: &str) -> Result<String> {
    let dir = opencode_config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine OpenCode config directory"))?;
    let path = dir.join("opencode.json");

    let Ok(body) = fs::read_to_string(&path) else {
        return Ok(format!("OpenCode plugin not registered: {module}"));
    };
    let mut json: serde_json::Value = serde_json::from_str(&body)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;

    let Some(arr) = json.get_mut("plugin").and_then(|p| p.as_array_mut()) else {
        return Ok(format!("OpenCode plugin not registered: {module}"));
    };
    let before = arr.len();
    arr.retain(|v| v.as_str() != Some(module));
    if arr.len() == before {
        return Ok(format!("OpenCode plugin not registered: {module}"));
    }

    fs::write(&path, serde_json::to_string_pretty(&json)?)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(format!("Deregistered OpenCode plugin: {module}"))
}

/// Set the OpenCode theme in the global config (`opencode.json` → `theme`).
///
/// OpenCode merges `config.json`, `opencode.json` and `opencode.jsonc` in that
/// order, so a `theme` in the jsonc file silently wins over what we write —
/// which is exactly the "I set a theme and nothing happened" failure. Report it
/// instead of writing a value that will not take effect. The jsonc file is not
/// parsed as JSON (it may hold comments); a textual probe is enough for a warning.
pub fn set_theme(theme: &str) -> Result<bool> {
    let dir = opencode_config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine OpenCode config directory"))?;

    let jsonc = dir.join("opencode.jsonc");
    if jsonc.exists()
        && fs::read_to_string(&jsonc)
            .is_ok_and(|c| c.contains("\"theme\"") && !c.contains(&format!("\"{theme}\"")))
    {
        anyhow::bail!(
            "{} already sets a different `theme` and is merged last — remove it or edit it by hand",
            jsonc.display()
        );
    }

    super::set_json_string(&dir.join("opencode.json"), &["theme"], theme)
}

/// Render one registry provider as an OpenCode `provider.<id>` object.
///
/// Secrets stay symbolic: `api_key_env` becomes OpenCode's native
/// `{env:VAR}` reference, and `base_url` is passed through verbatim
/// (OpenCode substitutes `{env:...}` itself at config load).
fn render_provider(id: &str, cfg: &crate::model::ProviderConfig) -> serde_json::Value {
    use serde_json::{Map, Value, json};

    let npm = cfg.npm.clone().unwrap_or_else(|| {
        match cfg.api() {
            crate::model::ProviderApi::Openai => "@ai-sdk/openai-compatible",
            crate::model::ProviderApi::Anthropic => "@ai-sdk/anthropic",
        }
        .to_string()
    });

    let mut options = Map::new();
    if let Some(base) = &cfg.base_url {
        options.insert("baseURL".into(), json!(base));
    }
    if let Some(var) = &cfg.api_key_env {
        options.insert("apiKey".into(), json!(format!("{{env:{var}}}")));
    }
    for (k, v) in &cfg.options {
        options.insert(k.clone(), v.clone());
    }

    let mut out = Map::new();
    out.insert("name".into(), json!(id));
    out.insert("npm".into(), json!(npm));
    out.insert("options".into(), Value::Object(options));

    let mut models = Map::new();
    for m in &cfg.models {
        let mut entry = Map::new();
        entry.insert("name".into(), json!(m.name));
        if let Some(limit) = m.limit.or(cfg.limit) {
            entry.insert("limit".into(), json!({ "context": limit }));
        }
        models.insert(m.id.clone(), Value::Object(entry));
    }
    if !models.is_empty() {
        out.insert("models".into(), Value::Object(models));
    }

    Value::Object(out)
}

/// Merge synced providers into an `opencode.json` root, replacing only the
/// `provider.<id>` keys muxix owns. Returns the ids whose entries changed.
fn merge_providers(
    root: &mut serde_json::Value,
    registry: &crate::model::ProviderRegistry,
) -> Result<Vec<String>> {
    let obj = root
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("opencode.json root is not an object"))?;
    let providers = obj
        .entry("provider".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("opencode.json `provider` is not an object"))?;

    let mut changed = Vec::new();
    for (id, cfg) in registry {
        if !cfg.has_connection() {
            continue;
        }
        let rendered = render_provider(id, cfg);
        if providers.get(id) != Some(&rendered) {
            providers.insert(id.clone(), rendered);
            changed.push(id.clone());
        }
    }
    Ok(changed)
}

/// Sync registry providers with connection details into the global
/// `opencode.json`. Non-destructive: only `provider.<id>` keys for synced ids
/// are touched. Returns human-readable result lines (empty = nothing to do).
pub fn sync_providers(
    registry: &crate::model::ProviderRegistry,
    dry_run: bool,
) -> Result<Vec<String>> {
    if !registry.values().any(|c| c.has_connection()) {
        return Ok(Vec::new());
    }
    let dir = opencode_config_dir()
        .ok_or_else(|| anyhow::anyhow!("Could not determine OpenCode config directory"))?;
    let path = dir.join("opencode.json");

    let mut root: serde_json::Value = if path.exists() {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("{} is not valid JSON", path.display()))?
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let changed = merge_providers(&mut root, registry)?;
    if changed.is_empty() {
        return Ok(Vec::new());
    }
    if dry_run {
        return Ok(changed
            .iter()
            .map(|id| format!("Would write provider `{id}` to {}", path.display()))
            .collect());
    }

    fs::create_dir_all(&dir).context("Failed to create OpenCode config directory")?;
    let output = serde_json::to_string_pretty(&root)?;
    fs::write(&path, output + "\n")
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(changed
        .iter()
        .map(|id| format!("Wrote provider `{id}` to {}", path.display()))
        .collect())
}

/// Install muxix plugin for OpenCode.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProviderConfig, ProviderModel, ProviderRegistry};
    use serde_json::json;

    fn litellm() -> ProviderConfig {
        ProviderConfig {
            base_url: Some("{env:LITELLM_BASE_URL}".into()),
            api_key_env: Some("LITELLM_API_KEY".into()),
            ..Default::default()
        }
    }

    #[test]
    fn merge_preserves_foreign_keys_and_renders_env_refs() {
        let mut root = json!({
            "theme": "catppuccin",
            "provider": { "corp": { "npm": "@ai-sdk/openai-compatible", "options": { "baseURL": "https://corp" } } }
        });
        let mut reg = ProviderRegistry::new();
        reg.insert("litellm".into(), litellm());

        let changed = merge_providers(&mut root, &reg).unwrap();
        assert_eq!(changed, vec!["litellm"]);
        assert_eq!(root["theme"], "catppuccin");
        assert_eq!(
            root["provider"]["corp"]["options"]["baseURL"],
            "https://corp"
        );
        let p = &root["provider"]["litellm"];
        assert_eq!(p["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(p["options"]["baseURL"], "{env:LITELLM_BASE_URL}");
        assert_eq!(p["options"]["apiKey"], "{env:LITELLM_API_KEY}");
    }

    #[test]
    fn merge_is_idempotent() {
        let mut root = json!({});
        let mut reg = ProviderRegistry::new();
        reg.insert("litellm".into(), litellm());
        assert!(!merge_providers(&mut root, &reg).unwrap().is_empty());
        assert!(merge_providers(&mut root, &reg).unwrap().is_empty());
    }

    #[test]
    fn registry_only_provider_not_written() {
        let mut root = json!({});
        let mut reg = ProviderRegistry::new();
        reg.insert(
            "anthropic".into(),
            ProviderConfig {
                limit: Some(200_000),
                ..Default::default()
            },
        );
        assert!(merge_providers(&mut root, &reg).unwrap().is_empty());
        assert!(
            root.get("provider")
                .is_none_or(|p| p.as_object().unwrap().is_empty())
        );
    }

    #[test]
    fn models_project_with_limits() {
        let mut root = json!({});
        let mut cfg = litellm();
        cfg.limit = Some(128_000);
        cfg.models = vec![ProviderModel {
            name: "sonnet".into(),
            id: "claude-sonnet-4-6".into(),
            tier: None,
            limit: None,
            compaction_limit: None,
        }];
        let mut reg = ProviderRegistry::new();
        reg.insert("litellm".into(), cfg);
        merge_providers(&mut root, &reg).unwrap();
        let m = &root["provider"]["litellm"]["models"]["claude-sonnet-4-6"];
        assert_eq!(m["name"], "sonnet");
        assert_eq!(m["limit"]["context"], 128_000);
    }
}
