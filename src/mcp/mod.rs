//! Reusable per-project MCP (Model Context Protocol) server registration.
//!
//! muxix reads MCP server declarations from the `mcp:` section of
//! `.muxix.yaml` and renders them into a project-level `.mcp.json` (Claude
//! Code's native format, `{ "mcpServers": { ... } }`). That file is propagated
//! into each worktree as a relative symlink (see `workflow::setup`), so any
//! agent that reads a project `.mcp.json` picks the servers up automatically.
//!
//! The render is a *merge*: server entries declared in `.muxix.yaml` are
//! upserted under `mcpServers`, while hand-added entries muxix doesn't manage
//! are preserved. A `x-muxix-managed` array tracks which keys muxix owns so
//! that removing/disabling a server in config also removes it from the file
//! without disturbing the user's own entries.
//!
//! This mechanism is intentionally generic (socraticode is just the first
//! consumer). Per-project harness status — configured servers, sync state, and
//! integration capabilities such as a code indexer — is exposed via
//! [`harness_status`] for the CLI (`muxix mcp status`) and the dashboard's
//! Project view, backed by the [`crate::project_state`] store.

use anyhow::{Context, Result};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::{Config, McpServerConfig};
use crate::project_state::{CapabilityStatus, ProjectStateStore};

pub mod targets;
pub use targets::{support_overview, sync_agent_mcp_configs};

/// The project-level MCP config filename (Claude Code's native format).
pub const MCP_JSON_FILENAME: &str = ".mcp.json";

/// Top-level key in `.mcp.json` tracking which server keys muxix manages.
const MANAGED_KEY: &str = "x-muxix-managed";

/// `project_state` fact recorded after a successful `.mcp.json` sync.
pub const MCP_SYNCED_FACT: &str = "mcp.synced";

/// Path to the project-level `.mcp.json` for `repo_root`.
pub fn mcp_json_path(repo_root: &Path) -> PathBuf {
    repo_root.join(MCP_JSON_FILENAME)
}

/// `project_state` capability name for a server's code-indexer integration.
///
/// The indexer itself (Docker/Qdrant bootstrap, `codebase_index`) is a separate
/// task; this just names the capability slot the Project view displays and the
/// future indexer action will acquire.
pub fn indexer_capability(server: &str) -> String {
    format!("indexer/{server}")
}

/// Render the `mcpServers` object for the given declarations.
/// Disabled servers are skipped. Returns a JSON object mapping name -> spec.
pub fn render_mcp_servers(servers: &BTreeMap<String, McpServerConfig>) -> Map<String, Value> {
    let mut out = Map::new();
    for (name, spec) in servers {
        if !spec.is_enabled() {
            continue;
        }
        let mut entry = Map::new();
        entry.insert("command".to_string(), Value::String(spec.command.clone()));
        if let Some(args) = &spec.args {
            entry.insert(
                "args".to_string(),
                Value::Array(args.iter().cloned().map(Value::String).collect()),
            );
        }
        if let Some(env) = &spec.env {
            let env_map: Map<String, Value> = env
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            entry.insert("env".to_string(), Value::Object(env_map));
        }
        out.insert(name.clone(), Value::Object(entry));
    }
    out
}

/// Merge declared servers into an existing `.mcp.json` value.
///
/// Preserves unmanaged (hand-added) servers, upserts muxix-managed ones, and
/// removes managed entries that are no longer declared/enabled. Idempotent:
/// applying it twice with the same input yields the same value.
pub fn merge_mcp_json(
    existing: Option<&Value>,
    servers: &BTreeMap<String, McpServerConfig>,
) -> Value {
    let rendered = render_mcp_servers(servers); // enabled only
    let new_managed: Vec<String> = rendered.keys().cloned().collect();

    let mut root: Map<String, Value> = existing
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let prev_managed: Vec<String> = root
        .get(MANAGED_KEY)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut mcp_servers: Map<String, Value> = root
        .get("mcpServers")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    // Drop previously-managed entries that are no longer managed (removed/disabled).
    for key in &prev_managed {
        if !new_managed.contains(key) {
            mcp_servers.remove(key);
        }
    }
    // Upsert the currently-managed entries.
    for (name, entry) in rendered {
        mcp_servers.insert(name, entry);
    }

    root.insert("mcpServers".to_string(), Value::Object(mcp_servers));
    if new_managed.is_empty() {
        root.remove(MANAGED_KEY);
    } else {
        root.insert(
            MANAGED_KEY.to_string(),
            Value::Array(new_managed.into_iter().map(Value::String).collect()),
        );
    }
    Value::Object(root)
}

/// Generate/refresh `<repo_root>/.mcp.json` from `config.mcp`.
///
/// Returns `Ok(Some(path))` when a file was written (servers declared), or
/// `Ok(None)` when there are no MCP servers configured (nothing to do).
/// Existing unmanaged server entries are preserved.
#[allow(dead_code)]
pub fn sync_mcp_json(repo_root: &Path, config: &Config) -> Result<Option<PathBuf>> {
    let servers = match &config.mcp {
        Some(m) if !m.is_empty() => m,
        _ => return Ok(None),
    };

    let path = mcp_json_path(repo_root);
    let existing: Option<Value> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok());

    let merged = merge_mcp_json(existing.as_ref(), servers);
    let mut content =
        serde_json::to_string_pretty(&merged).context("Failed to serialize .mcp.json")?;
    content.push('\n');
    std::fs::write(&path, content)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(Some(path))
}

/// Record in `project_state` that the project's MCP config has been synced.
pub fn record_synced(project_dir: &Path) -> Result<()> {
    ProjectStateStore::open(project_dir)?.set_fact(MCP_SYNCED_FACT, "true")?;
    Ok(())
}

/// Status of one MCP server, for display in the CLI and Project view.
pub struct McpServerStatus {
    pub name: String,
    /// Rendered command line (e.g. `npx -y socraticode`).
    pub command: String,
    pub enabled: bool,
}

/// Status of one harness integration capability (e.g. a code indexer).
pub struct IntegrationStatus {
    /// Capability name (e.g. `indexer/socraticode`).
    pub name: String,
    pub status: CapabilityStatus,
}

/// Aggregate per-project harness status for the Project view and `mcp status`.
pub struct HarnessStatus {
    #[allow(dead_code)]
    pub repo_root: PathBuf,
    pub mcp_json_exists: bool,
    pub mcp_synced: bool,
    pub servers: Vec<McpServerStatus>,
    pub integrations: Vec<IntegrationStatus>,
}

/// Render a server's command line for display (e.g. `npx -y socraticode`).
fn render_command_line(spec: &McpServerConfig) -> String {
    match &spec.args {
        Some(args) if !args.is_empty() => format!("{} {}", spec.command, args.join(" ")),
        _ => spec.command.clone(),
    }
}

/// Build the harness status for `repo_root` from config + project_state.
///
/// Reads are tolerant: a missing/empty project_state yields `Absent`/false
/// rather than an error.
pub fn harness_status(repo_root: &Path, config: &Config) -> Result<HarnessStatus> {
    let servers: Vec<McpServerStatus> = config
        .mcp
        .as_ref()
        .map(|m| {
            m.iter()
                .map(|(name, spec)| McpServerStatus {
                    name: name.clone(),
                    command: render_command_line(spec),
                    enabled: spec.is_enabled(),
                })
                .collect()
        })
        .unwrap_or_default();

    let store = ProjectStateStore::open(repo_root)?;
    let mcp_synced = store.get_fact(MCP_SYNCED_FACT)?.as_deref() == Some("true");

    let integrations = servers
        .iter()
        .map(|s| {
            let cap = indexer_capability(&s.name);
            let status = store
                .get_capability(&cap)
                .map(|c| c.status)
                .unwrap_or(CapabilityStatus::Absent);
            IntegrationStatus { name: cap, status }
        })
        .collect();

    Ok(HarnessStatus {
        repo_root: repo_root.to_path_buf(),
        mcp_json_exists: mcp_json_path(repo_root).exists(),
        mcp_synced,
        servers,
        integrations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(command: &str, args: &[&str]) -> McpServerConfig {
        McpServerConfig {
            command: command.to_string(),
            args: Some(args.iter().map(|s| s.to_string()).collect()),
            env: None,
            enabled: None,
            agents: None,
            requires: None,
        }
    }

    fn socraticode_map() -> BTreeMap<String, McpServerConfig> {
        let mut m = BTreeMap::new();
        m.insert(
            "socraticode".to_string(),
            server("npx", &["-y", "socraticode"]),
        );
        m
    }

    #[test]
    fn renders_command_and_args() {
        let rendered = render_mcp_servers(&socraticode_map());
        let entry = rendered.get("socraticode").unwrap();
        assert_eq!(entry["command"], Value::String("npx".into()));
        assert_eq!(
            entry["args"],
            Value::Array(vec!["-y".into(), "socraticode".into()])
        );
    }

    #[test]
    fn disabled_servers_are_skipped() {
        let mut m = socraticode_map();
        m.get_mut("socraticode").unwrap().enabled = Some(false);
        assert!(render_mcp_servers(&m).is_empty());
    }

    #[test]
    fn merge_from_empty_produces_mcp_servers_and_managed_marker() {
        let merged = merge_mcp_json(None, &socraticode_map());
        assert!(merged["mcpServers"]["socraticode"].is_object());
        assert_eq!(
            merged[MANAGED_KEY],
            Value::Array(vec!["socraticode".into()])
        );
    }

    #[test]
    fn merge_is_idempotent() {
        let first = merge_mcp_json(None, &socraticode_map());
        let second = merge_mcp_json(Some(&first), &socraticode_map());
        assert_eq!(first, second);
    }

    #[test]
    fn merge_preserves_unmanaged_servers() {
        let existing: Value = serde_json::json!({
            "mcpServers": { "other": { "command": "node", "args": ["other.js"] } }
        });
        let merged = merge_mcp_json(Some(&existing), &socraticode_map());
        // Hand-added "other" survives; managed "socraticode" is added.
        assert!(merged["mcpServers"]["other"].is_object());
        assert!(merged["mcpServers"]["socraticode"].is_object());
        // Only socraticode is tracked as managed.
        assert_eq!(
            merged[MANAGED_KEY],
            Value::Array(vec!["socraticode".into()])
        );
    }

    #[test]
    fn sync_mcp_json_writes_file_and_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config {
            mcp: Some(socraticode_map()),
            ..Default::default()
        };

        let path = sync_mcp_json(dir.path(), &config).unwrap().unwrap();
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.ends_with('\n'));
        let parsed: Value = serde_json::from_str(&first).unwrap();
        assert!(parsed["mcpServers"]["socraticode"].is_object());

        // Re-sync yields byte-identical output.
        sync_mcp_json(dir.path(), &config).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
    }

    #[test]
    fn sync_mcp_json_noop_without_servers() {
        let dir = tempfile::TempDir::new().unwrap();
        let config = Config::default();
        assert!(sync_mcp_json(dir.path(), &config).unwrap().is_none());
        assert!(!mcp_json_path(dir.path()).exists());
    }

    #[test]
    fn removing_a_managed_server_cleans_it_up_but_keeps_unmanaged() {
        let existing = merge_mcp_json(
            Some(&serde_json::json!({
                "mcpServers": { "other": { "command": "node" } }
            })),
            &socraticode_map(),
        );
        // Now sync with no declared servers: socraticode removed, other kept.
        let empty: BTreeMap<String, McpServerConfig> = BTreeMap::new();
        let merged = merge_mcp_json(Some(&existing), &empty);
        assert!(merged["mcpServers"]["other"].is_object());
        assert!(merged["mcpServers"].get("socraticode").is_none());
        assert!(merged.get(MANAGED_KEY).is_none());
    }
}
