//! Agent-agnostic MCP config targets.
//!
//! A single `mcp:` declaration in `.muxix.yaml` is the source of truth.
//! Each coding agent, however, reads MCP servers from its own native config
//! file in its own format. An [`McpTarget`] knows, for one agent, *where* that
//! file lives and *how* to merge the declared servers into it without
//! disturbing entries the user added by hand.
//!
//! Every agent has a target. Claude, pi, omp and Copilot share the project
//! `.mcp.json` and Gemini its `.gemini/settings.json` — all the
//! `{ "mcpServers": { … } }` shape, via [`super::merge_mcp_json`]. OpenCode has
//! its own `mcp` schema in `opencode.json`. Codex is the one non-JSON store:
//! a marker-delimited region of `[mcp_servers.*]` tables in the project's
//! `.codex/config.toml` layer, written through [`McpTarget::render`].

use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::merge_mcp_json;
use crate::agent::setup::{self, Agent};
use crate::config::{Config, McpServerConfig};

/// One agent's native MCP config file: where it is and how to merge into it.
pub trait McpTarget {
    /// The agent this target writes config for.
    fn agent(&self) -> Agent;

    /// Project-level path to this agent's MCP config file under `repo_root`.
    fn config_path(&self, repo_root: &Path) -> PathBuf;

    /// Merge the declared servers into existing parsed config (format-specific).
    /// Must be idempotent and preserve unmanaged (hand-added) entries.
    fn merge(&self, existing: Option<&Value>, servers: &BTreeMap<String, McpServerConfig>)
    -> Value;

    /// Render the whole file body to write, given its current text.
    ///
    /// JSON targets get this for free from [`McpTarget::merge`]. A target whose
    /// native config is not JSON (Codex: TOML) overrides this instead and
    /// leaves `merge` unused — the text seam is what lets one `sync_target`
    /// drive both.
    fn render(
        &self,
        existing: Option<&str>,
        servers: &BTreeMap<String, McpServerConfig>,
    ) -> Result<String> {
        let parsed: Option<Value> = existing.and_then(|s| serde_json::from_str(s).ok());
        let merged = self.merge(parsed.as_ref(), servers);
        Ok(format!("{}\n", serde_json::to_string_pretty(&merged)?))
    }

    /// Pre-approve the named servers in this agent's *native* trust mechanism so
    /// a harness-launched agent doesn't block on an interactive "trust this MCP
    /// server?" prompt at startup. Each agent does this its own way (Claude:
    /// `enabledMcpjsonServers` in `~/.claude/settings.json`; Gemini: per-server
    /// `trust`). Default is a no-op for agents with no such gate. Idempotent.
    fn approve_servers(&self, _repo_root: &Path, _server_names: &[&str]) -> Result<()> {
        Ok(())
    }
}

/// Claude Code: project-root `.mcp.json`, `{ "mcpServers": { … } }`.
pub struct ClaudeTarget;

impl McpTarget for ClaudeTarget {
    fn agent(&self) -> Agent {
        Agent::Claude
    }
    fn config_path(&self, repo_root: &Path) -> PathBuf {
        repo_root.join(super::MCP_JSON_FILENAME)
    }
    fn merge(
        &self,
        existing: Option<&Value>,
        servers: &BTreeMap<String, McpServerConfig>,
    ) -> Value {
        merge_mcp_json(existing, servers)
    }
    fn approve_servers(&self, _repo_root: &Path, server_names: &[&str]) -> Result<()> {
        // Claude's trust gate lives in user-global ~/.claude/settings.json, which
        // applies across all of the project's worktrees (the prompt is keyed by
        // path, so per-worktree project settings would re-prompt).
        setup::claude::approve_mcp_servers(server_names)
    }
}

/// Gemini CLI: project-root `.gemini/settings.json`, same `mcpServers` shape as
/// Claude (other settings keys in the file are preserved by the merge).
pub struct GeminiTarget;

impl McpTarget for GeminiTarget {
    fn agent(&self) -> Agent {
        Agent::Gemini
    }
    fn config_path(&self, repo_root: &Path) -> PathBuf {
        repo_root.join(".gemini").join("settings.json")
    }
    fn merge(
        &self,
        existing: Option<&Value>,
        servers: &BTreeMap<String, McpServerConfig>,
    ) -> Value {
        let mut merged = merge_mcp_json(existing, servers);
        // Keep the `trust: true` that `approve_servers` wrote last run; otherwise
        // merge strips it and every sync rewrites the file ("updated" forever).
        if let (Some(prev), Some(cur)) = (
            existing.and_then(|v| v.get("mcpServers")).and_then(|v| v.as_object()),
            merged.get_mut("mcpServers").and_then(|v| v.as_object_mut()),
        ) {
            for (name, entry) in cur.iter_mut() {
                if prev.get(name).and_then(|e| e.get("trust")) == Some(&Value::Bool(true)) {
                    if let Some(obj) = entry.as_object_mut() {
                        obj.insert("trust".to_string(), Value::Bool(true));
                    }
                }
            }
        }
        merged
    }
    fn approve_servers(&self, repo_root: &Path, server_names: &[&str]) -> Result<()> {
        // Gemini marks an MCP server trusted via a per-server `trust: true` in its
        // own `.gemini/settings.json` (the file `merge` just wrote), which skips
        // that server's tool-call confirmations.
        if server_names.is_empty() {
            return Ok(());
        }
        let path = self.config_path(repo_root);
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Ok(());
        };
        let Ok(mut v) = serde_json::from_str::<Value>(&content) else {
            return Ok(());
        };
        let Some(servers) = v.get_mut("mcpServers").and_then(|s| s.as_object_mut()) else {
            return Ok(());
        };
        for name in server_names {
            if let Some(entry) = servers.get_mut(*name).and_then(|e| e.as_object_mut()) {
                entry.insert("trust".to_string(), Value::Bool(true));
            }
        }
        let mut out = serde_json::to_string_pretty(&v)
            .with_context(|| format!("Failed to serialize {}", path.display()))?;
        out.push('\n');
        std::fs::write(&path, out)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }
}

/// Pi: project-root `.mcp.json` via pi-mcp-adapter, same `mcpServers` shape as Claude.
/// pi-mcp-adapter reads `.mcp.json` directly, so this target shares the file and merge
/// logic with [`ClaudeTarget`] — no separate approval gate needed.
pub struct PiTarget;

impl McpTarget for PiTarget {
    fn agent(&self) -> Agent {
        Agent::Pi
    }
    fn config_path(&self, repo_root: &Path) -> PathBuf {
        repo_root.join(super::MCP_JSON_FILENAME)
    }
    fn merge(
        &self,
        existing: Option<&Value>,
        servers: &BTreeMap<String, McpServerConfig>,
    ) -> Value {
        merge_mcp_json(existing, servers)
    }
}

/// omp (oh-my-pi): pi-compatible, reads project-root `.mcp.json` via
/// pi-mcp-adapter — same shape and merge logic as [`PiTarget`].
pub struct OmpTarget;

impl McpTarget for OmpTarget {
    fn agent(&self) -> Agent {
        Agent::Omp
    }
    fn config_path(&self, repo_root: &Path) -> PathBuf {
        repo_root.join(super::MCP_JSON_FILENAME)
    }
    fn merge(
        &self,
        existing: Option<&Value>,
        servers: &BTreeMap<String, McpServerConfig>,
    ) -> Value {
        merge_mcp_json(existing, servers)
    }
}

/// Copilot CLI: project-root `.mcp.json`, the same `{ "mcpServers": { … } }`
/// shape Claude uses — Copilot reads that file (and `.github/mcp.json`) as its
/// project-level MCP config, so this target converges the same file Claude/pi/omp
/// already write and `sync_agent_mcp_configs` reports it once.
///
/// Tool approval lives in Copilot's own `permissions-config.json`, keyed by
/// project path and written by the CLI itself; muxix does not forge entries
/// there, so `approve_servers` stays the no-op default.
pub struct CopilotTarget;

impl McpTarget for CopilotTarget {
    fn agent(&self) -> Agent {
        Agent::Copilot
    }
    fn config_path(&self, repo_root: &Path) -> PathBuf {
        repo_root.join(super::MCP_JSON_FILENAME)
    }
    fn merge(
        &self,
        existing: Option<&Value>,
        servers: &BTreeMap<String, McpServerConfig>,
    ) -> Value {
        merge_mcp_json(existing, servers)
    }
}

/// Codex: the project config layer `.codex/config.toml`, TOML, with the servers
/// in a muxix-managed marker region of `[mcp_servers.*]` tables.
///
/// TOML has no merge-patch, so this target overrides `render` (text in, text
/// out) and leaves `merge` unused; everything outside the markers — including
/// hand-written `[mcp_servers.*]` entries — is preserved byte-for-byte.
pub struct CodexTarget;

impl McpTarget for CodexTarget {
    fn agent(&self) -> Agent {
        Agent::Codex
    }
    fn config_path(&self, repo_root: &Path) -> PathBuf {
        repo_root.join(".codex").join("config.toml")
    }
    fn merge(
        &self,
        _existing: Option<&Value>,
        _servers: &BTreeMap<String, McpServerConfig>,
    ) -> Value {
        // Unused: `render` owns this target's TOML body.
        Value::Null
    }
    fn render(
        &self,
        existing: Option<&str>,
        servers: &BTreeMap<String, McpServerConfig>,
    ) -> Result<String> {
        let content = existing.unwrap_or_default();
        let region = setup::codex::render_mcp_region(servers);
        Ok(setup::codex::splice_mcp_region(content, &region)
            .unwrap_or_else(|| content.to_string()))
    }
    fn approve_servers(&self, repo_root: &Path, _server_names: &[&str]) -> Result<()> {
        // Codex reads a project `.codex/config.toml` layer only for a project it
        // trusts, so without this the file above is inert. Failing loudly beats
        // leaving an unread config behind.
        setup::codex::trust_project(repo_root, false).map(|_| ())
    }
}

/// OpenCode: project-root `opencode.json`, `{ "mcp": { name: { type, command[], enabled } } }`.
/// Different schema from Claude/Gemini — `command` is an argv array and servers
/// carry `type`/`enabled`. OpenCode has no interactive startup trust prompt:
/// `enabled: true` in the config IS the approval, so `approve_servers` is the
/// trait default (no-op).
pub struct OpenCodeTarget;

impl McpTarget for OpenCodeTarget {
    fn agent(&self) -> Agent {
        Agent::OpenCode
    }
    fn config_path(&self, repo_root: &Path) -> PathBuf {
        repo_root.join("opencode.json")
    }
    fn merge(
        &self,
        existing: Option<&Value>,
        servers: &BTreeMap<String, McpServerConfig>,
    ) -> Value {
        merge_opencode_mcp(existing, servers)
    }
}

/// Merge declared servers into an OpenCode `opencode.json`, upserting enabled
/// servers under `mcp` in OpenCode's local-server shape and preserving every
/// other key (including hand-added MCP entries).
fn merge_opencode_mcp(
    existing: Option<&Value>,
    servers: &BTreeMap<String, McpServerConfig>,
) -> Value {
    let mut root = existing
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut mcp = root
        .get("mcp")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    for (name, spec) in servers {
        if !spec.is_enabled() {
            continue;
        }
        let mut command = vec![Value::String(spec.command.clone())];
        if let Some(args) = &spec.args {
            command.extend(args.iter().cloned().map(Value::String));
        }
        let mut entry = serde_json::Map::new();
        entry.insert("type".to_string(), Value::String("local".to_string()));
        entry.insert("command".to_string(), Value::Array(command));
        entry.insert("enabled".to_string(), Value::Bool(true));
        if let Some(env) = &spec.env {
            let env_map: serde_json::Map<String, Value> = env
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            entry.insert("environment".to_string(), Value::Object(env_map));
        }
        mcp.insert(name.clone(), Value::Object(entry));
    }

    root.insert("mcp".to_string(), Value::Object(mcp));
    Value::Object(root)
}

/// One coding agent's MCP support — **exhaustive** over [`Agent`], so adding a
/// new agent (or switching to one like `pi`) forces a decision here and shows up
/// in `muxix mcp status` instead of silently doing nothing.
pub enum McpSupport {
    /// muxix writes this agent's MCP config and pre-approves the servers.
    Supported(Box<dyn McpTarget>),
    /// No muxix MCP adapter yet — carries a short TODO reason for visibility.
    Unsupported(&'static str),
}

/// The MCP support for a given agent. Exhaustive match: a future `Agent` variant
/// won't compile until its MCP support is declared.
pub fn mcp_support(agent: Agent) -> McpSupport {
    match agent {
        Agent::Claude => McpSupport::Supported(Box::new(ClaudeTarget)),
        Agent::OpenCode => McpSupport::Supported(Box::new(OpenCodeTarget)),
        Agent::Gemini => McpSupport::Supported(Box::new(GeminiTarget)),
        Agent::Codex => McpSupport::Supported(Box::new(CodexTarget)),
        Agent::Copilot => McpSupport::Supported(Box::new(CopilotTarget)),
        Agent::Pi => McpSupport::Supported(Box::new(PiTarget)),
        Agent::Omp => McpSupport::Supported(Box::new(OmpTarget)),
    }
}

/// All MCP targets muxix can render today, derived from [`mcp_support`] over
/// every agent (so the set is never out of sync with the typed declaration).
pub fn all_targets() -> Vec<Box<dyn McpTarget>> {
    Agent::ALL
        .into_iter()
        .filter_map(|a| match mcp_support(a) {
            McpSupport::Supported(t) => Some(t),
            McpSupport::Unsupported(_) => None,
        })
        .collect()
}

/// Per-agent MCP support for display (`muxix mcp status`): `(agent, Ok(()) |
/// Err(reason))`. Lists EVERY agent so unimplemented ones are visible.
pub fn support_overview() -> Vec<(Agent, Result<(), &'static str>)> {
    Agent::ALL
        .into_iter()
        .map(|a| match mcp_support(a) {
            McpSupport::Supported(_) => (a, Ok(())),
            McpSupport::Unsupported(reason) => (a, Err(reason)),
        })
        .collect()
}

/// Write one target's config from `servers` (no-op when there are no servers).
///
/// Reads the existing file (if any), merges, and writes pretty JSON with a
/// trailing newline, creating parent directories as needed. Returns the path
/// when written.
pub fn sync_target(
    target: &dyn McpTarget,
    repo_root: &Path,
    servers: &BTreeMap<String, McpServerConfig>,
) -> Result<Option<PathBuf>> {
    if servers.is_empty() {
        return Ok(None);
    }
    let path = target.config_path(repo_root);

    // A broken symlink (symlink exists but target missing) causes fs::write to
    // fail. Replace it with a regular file so this worktree owns its own copy.
    if path.is_symlink() && !path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Failed to remove broken symlink {}", path.display()))?;
    }

    let raw = std::fs::read_to_string(&path).ok();
    let content = target
        .render(raw.as_deref(), servers)
        .with_context(|| format!("Failed to render {}", path.display()))?;
    // Converged already: no write, no "updated" report, `--check` stays quiet.
    if raw.as_deref() == Some(content.as_str()) {
        return Ok(None);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    std::fs::write(&path, content)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(Some(path))
}

/// Render the unified `mcp:` config into every relevant agent's native format.
///
/// Claude's `.mcp.json` is always written (back-compat: it is also the file
/// symlinked into worktrees). Other agents are written only when their CLI is
/// detected *or* their config file already exists — so we don't litter configs
/// for agents the user doesn't use. Returns the paths written.
pub fn sync_agent_mcp_configs(repo_root: &Path, config: &Config) -> Result<Vec<PathBuf>> {
    let servers = match &config.mcp {
        Some(m) if !m.is_empty() => m,
        _ => return Ok(Vec::new()),
    };

    let mut written: Vec<PathBuf> = Vec::new();
    for target in all_targets() {
        // Claude/pi/omp share `.mcp.json`; once it is converged for one of them
        // the others' merge is a no-op, and the report must not list it thrice.
        if written.contains(&target.config_path(repo_root)) {
            continue;
        }
        let is_claude = target.agent() == Agent::Claude;
        let wanted = is_claude
            || setup::is_detected(target.agent())
            || target.config_path(repo_root).exists();
        if !wanted {
            continue;
        }
        // Per-agent scoping: only servers whose `agents:` list (if any) matches
        // this target. An empty result skips the target entirely (`sync_target`
        // no-ops), so no empty managed block is created.
        let scoped: BTreeMap<String, McpServerConfig> = servers
            .iter()
            .filter(|(_, s)| s.applies_to(target.agent()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let enabled_names: Vec<&str> = scoped
            .iter()
            .filter(|(_, s)| s.is_enabled())
            .map(|(name, _)| name.as_str())
            .collect();
        if let Some(path) = sync_target(target.as_ref(), repo_root, &scoped)? {
            written.push(path);
            // Pre-approve in this agent's native trust mechanism (best-effort:
            // a trust-write failure must not abort the config sync).
            if let Err(e) = target.approve_servers(repo_root, &enabled_names) {
                eprintln!(
                    "warn: could not pre-approve MCP servers for {:?}: {e}",
                    target.agent()
                );
            }
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn socraticode_map() -> BTreeMap<String, McpServerConfig> {
        let mut m = BTreeMap::new();
        m.insert(
            "socraticode".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: Some(vec!["-y".to_string(), "socraticode".to_string()]),
                env: None,
                enabled: None,
                agents: None,
                requires: None,
            },
        );
        m
    }

    #[test]
    fn claude_target_path_and_shape() {
        let t = ClaudeTarget;
        assert_eq!(
            t.config_path(Path::new("/repo")),
            Path::new("/repo/.mcp.json")
        );
        let merged = t.merge(None, &socraticode_map());
        assert!(merged["mcpServers"]["socraticode"].is_object());
    }

    #[test]
    fn gemini_target_path_and_shape() {
        let t = GeminiTarget;
        assert_eq!(
            t.config_path(Path::new("/repo")),
            Path::new("/repo/.gemini/settings.json")
        );
        let merged = t.merge(None, &socraticode_map());
        assert!(merged["mcpServers"]["socraticode"].is_object());
    }

    #[test]
    fn gemini_merge_preserves_unmanaged_settings() {
        let t = GeminiTarget;
        let existing = serde_json::json!({
            "theme": "Default",
            "mcpServers": { "other": { "command": "node" } }
        });
        let merged = t.merge(Some(&existing), &socraticode_map());
        // User's own settings and hand-added servers survive.
        assert_eq!(merged["theme"], Value::String("Default".into()));
        assert!(merged["mcpServers"]["other"].is_object());
        assert!(merged["mcpServers"]["socraticode"].is_object());
    }

    #[test]
    fn sync_target_writes_into_nested_dir() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = sync_target(&GeminiTarget, dir.path(), &socraticode_map())
            .unwrap()
            .unwrap();
        assert!(path.ends_with(".gemini/settings.json"));
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.ends_with('\n'));
        let parsed: Value = serde_json::from_str(&content).unwrap();
        assert!(parsed["mcpServers"]["socraticode"].is_object());
    }

    #[test]
    fn gemini_approve_marks_managed_servers_trusted() {
        let dir = tempfile::TempDir::new().unwrap();
        // Sync writes the server, then approve marks it trusted.
        sync_target(&GeminiTarget, dir.path(), &socraticode_map()).unwrap();
        GeminiTarget
            .approve_servers(dir.path(), &["socraticode"])
            .unwrap();

        let path = dir.path().join(".gemini").join("settings.json");
        let parsed: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed["mcpServers"]["socraticode"]["trust"],
            Value::Bool(true)
        );

        // Idempotent: a second approve keeps a single boolean, not nested.
        GeminiTarget
            .approve_servers(dir.path(), &["socraticode"])
            .unwrap();
        let parsed2: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            parsed2["mcpServers"]["socraticode"]["trust"],
            Value::Bool(true)
        );
    }

    #[test]
    fn opencode_target_writes_local_mcp_schema() {
        let t = OpenCodeTarget;
        assert_eq!(
            t.config_path(Path::new("/repo")),
            Path::new("/repo/opencode.json")
        );
        let merged = t.merge(None, &socraticode_map());
        let entry = &merged["mcp"]["socraticode"];
        assert_eq!(entry["type"], Value::String("local".into()));
        assert_eq!(entry["enabled"], Value::Bool(true));
        // command is an argv array: [command, ...args].
        assert_eq!(
            entry["command"],
            Value::Array(vec!["npx".into(), "-y".into(), "socraticode".into()])
        );
    }

    #[test]
    fn opencode_merge_preserves_unmanaged_keys_and_servers() {
        let existing = serde_json::json!({
            "theme": "dark",
            "mcp": { "other": { "type": "local", "command": ["node"], "enabled": true } }
        });
        let merged = OpenCodeTarget.merge(Some(&existing), &socraticode_map());
        assert_eq!(merged["theme"], Value::String("dark".into()));
        assert!(merged["mcp"]["other"].is_object());
        assert!(merged["mcp"]["socraticode"].is_object());
    }

    #[test]
    fn every_agent_has_an_mcp_target() {
        // Every agent now has an adapter; `all_targets()` must list them all and
        // no agent may report an "unsupported" reason.
        let supported: Vec<Agent> = all_targets().iter().map(|t| t.agent()).collect();
        for agent in Agent::ALL {
            assert!(supported.contains(&agent), "{} has no MCP target", agent.name());
        }
        for (agent, support) in support_overview() {
            assert!(support.is_ok(), "{} reported unsupported", agent.name());
        }
    }

    #[test]
    fn copilot_shares_the_project_mcp_json() {
        let t = CopilotTarget;
        assert_eq!(
            t.config_path(Path::new("/repo")),
            Path::new("/repo/.mcp.json"),
            "Copilot reads the same project file Claude does"
        );
        let merged = t.merge(None, &socraticode_map());
        assert!(merged["mcpServers"]["socraticode"].is_object());
    }

    #[test]
    fn codex_renders_a_managed_toml_region() {
        let t = CodexTarget;
        assert_eq!(
            t.config_path(Path::new("/repo")),
            Path::new("/repo/.codex/config.toml")
        );

        // Hand-written content outside the markers survives.
        let existing = "model = \"gpt-5\"\n";
        let body = t.render(Some(existing), &socraticode_map()).unwrap();
        assert!(body.starts_with("model = \"gpt-5\""), "{body}");
        assert!(body.contains("[mcp_servers.socraticode]"), "{body}");
        assert!(body.contains("command = \"npx\""), "{body}");
        assert!(body.contains("args = [\"-y\", \"socraticode\"]"), "{body}");

        // Idempotent: rendering the same servers over the result is a fixpoint.
        let again = t.render(Some(&body), &socraticode_map()).unwrap();
        assert_eq!(again, body);

        // Dropping a server removes its table but keeps the rest of the file.
        let emptied = t.render(Some(&body), &BTreeMap::new()).unwrap();
        assert!(!emptied.contains("mcp_servers"), "{emptied}");
        assert!(emptied.contains("model = \"gpt-5\""), "{emptied}");
    }

    #[test]
    fn approve_noop_when_no_config_file() {
        let dir = tempfile::TempDir::new().unwrap();
        // No .gemini/settings.json yet → approve is a silent no-op, not an error.
        assert!(
            GeminiTarget
                .approve_servers(dir.path(), &["socraticode"])
                .is_ok()
        );
    }

    #[test]
    fn sync_target_noop_without_servers() {
        let dir = tempfile::TempDir::new().unwrap();
        let empty: BTreeMap<String, McpServerConfig> = BTreeMap::new();
        assert!(
            sync_target(&ClaudeTarget, dir.path(), &empty)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn sync_agent_configs_always_writes_claude_and_existing_gemini() {
        let dir = tempfile::TempDir::new().unwrap();
        // Pre-create a Gemini settings file so it counts as "wanted" without
        // depending on whether the gemini CLI is installed on the test host.
        let gemini_path = dir.path().join(".gemini").join("settings.json");
        std::fs::create_dir_all(gemini_path.parent().unwrap()).unwrap();
        std::fs::write(&gemini_path, "{}\n").unwrap();

        let config = Config {
            mcp: Some(socraticode_map()),
            ..Default::default()
        };
        let written = sync_agent_mcp_configs(dir.path(), &config).unwrap();

        let claude_path = dir.path().join(super::super::MCP_JSON_FILENAME);
        assert!(
            written.contains(&claude_path),
            "Claude .mcp.json always written"
        );
        assert!(
            written.contains(&gemini_path),
            "existing Gemini settings written"
        );

        // Both files now carry the managed server.
        for p in [&claude_path, &gemini_path] {
            let parsed: Value = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
            assert!(parsed["mcpServers"]["socraticode"].is_object());
        }
    }

    #[test]
    fn sync_agent_configs_scopes_servers_per_agent() {
        let dir = tempfile::TempDir::new().unwrap();
        // Pre-create opencode.json and .gemini/settings.json so both targets are
        // "wanted" regardless of which CLIs the test host has installed.
        std::fs::write(dir.path().join("opencode.json"), "{}\n").unwrap();
        let gemini_path = dir.path().join(".gemini").join("settings.json");
        std::fs::create_dir_all(gemini_path.parent().unwrap()).unwrap();
        std::fs::write(&gemini_path, "{}\n").unwrap();

        let mut servers = socraticode_map();
        servers.insert(
            "taskflow".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: Some(vec![
                    "-y".to_string(),
                    "-p".to_string(),
                    "opencode-taskflow".to_string(),
                    "opencode-taskflow-mcp".to_string(),
                ]),
                env: None,
                enabled: None,
                agents: Some(vec!["opencode".to_string()]),
                requires: None,
            },
        );
        let config = Config {
            mcp: Some(servers),
            ..Default::default()
        };
        sync_agent_mcp_configs(dir.path(), &config).unwrap();

        // OpenCode sees both servers; Claude and Gemini only the unscoped one.
        let oc: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("opencode.json")).unwrap(),
        )
        .unwrap();
        assert!(oc["mcp"]["taskflow"].is_object());
        assert!(oc["mcp"]["socraticode"].is_object());
        for p in [
            dir.path().join(super::super::MCP_JSON_FILENAME),
            gemini_path,
        ] {
            let parsed: Value =
                serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
            assert!(parsed["mcpServers"]["socraticode"].is_object());
            assert!(parsed["mcpServers"]["taskflow"].is_null(), "{}", p.display());
        }
    }

    #[test]
    fn sync_agent_configs_skips_target_when_all_servers_scoped_away() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut servers = socraticode_map();
        for s in servers.values_mut() {
            s.agents = Some(vec!["opencode".to_string()]);
        }
        let config = Config {
            mcp: Some(servers),
            ..Default::default()
        };
        let written = sync_agent_mcp_configs(dir.path(), &config).unwrap();
        // Claude is always "wanted", but every server is scoped away → no file.
        assert!(!dir.path().join(super::super::MCP_JSON_FILENAME).exists());
        assert!(!written.iter().any(|p| p.ends_with(".mcp.json")));
    }

    #[test]
    fn applies_to_matches_config_keys() {
        let mut s = socraticode_map().remove("socraticode").unwrap();
        assert!(s.applies_to(Agent::Claude));
        s.agents = Some(vec!["claude code".to_string(), "pi".to_string()]);
        assert!(s.applies_to(Agent::Claude));
        assert!(s.applies_to(Agent::Pi));
        assert!(!s.applies_to(Agent::OpenCode));
    }

    #[test]
    fn sync_agent_configs_noop_without_servers() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config::default();
        assert!(
            sync_agent_mcp_configs(dir.path(), &config)
                .unwrap()
                .is_empty()
        );
    }
}
