//! Core data structures for filesystem-based state storage.

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Characters that need encoding in filenames (beyond control chars).
/// Includes path separators and other filesystem-unsafe characters.
pub(crate) const FILENAME_ENCODE_SET: &AsciiSet =
    &CONTROLS.add(b'/').add(b'\\').add(b':').add(b'%');

use crate::multiplexer::types::{AgentPane, AgentStatus};

fn new_agent_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Composite pane identifier for unique state file naming.
///
/// Combines backend type, instance identifier, and pane ID to create
/// a globally unique key that works across multiple terminal multiplexer
/// instances.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct PaneKey {
    /// Backend type: "tmux", "wezterm", "zellij"
    pub backend: String,

    /// Backend instance identifier (e.g., tmux socket path, wezterm mux ID)
    pub instance: String,

    /// Pane identifier within the backend.
    /// - tmux: pane ID (e.g., "%42")
    /// - WezTerm: numeric pane ID
    /// - Zellij: terminal pane ID (e.g., "terminal_5")
    pub pane_id: String,
}

impl PaneKey {
    /// Generate filename for this pane's state file.
    ///
    /// Format: `{backend}__{instance}__{pane_id}.json`
    /// Double underscores used since pane IDs may contain single underscores.
    /// Filesystem-unsafe characters are percent-encoded for safety.
    pub fn to_filename(&self) -> String {
        let safe_instance = utf8_percent_encode(&self.instance, FILENAME_ENCODE_SET).to_string();
        let safe_pane_id = utf8_percent_encode(&self.pane_id, FILENAME_ENCODE_SET).to_string();
        format!("{}__{}__{}.json", self.backend, safe_instance, safe_pane_id)
    }

    /// Parse a PaneKey from a filename.
    ///
    /// Returns None if the filename doesn't match the expected format.
    #[allow(dead_code)] // Used in tests, may be used in future features
    pub fn from_filename(filename: &str) -> Option<Self> {
        let stem = filename.strip_suffix(".json")?;
        let parts: Vec<&str> = stem.splitn(3, "__").collect();
        if parts.len() == 3 {
            Some(PaneKey {
                backend: parts[0].to_string(),
                instance: percent_decode_str(parts[1])
                    .decode_utf8_lossy()
                    .into_owned(),
                pane_id: percent_decode_str(parts[2])
                    .decode_utf8_lossy()
                    .into_owned(),
            })
        } else {
            None
        }
    }
}

/// Per-agent state stored as one JSON file per agent.
///
/// This is the persistent storage format. For dashboard display,
/// convert to `AgentPane` using `to_agent_pane()`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentState {
    /// Stable unique identifier for this agent instance (UUID v4).
    ///
    /// Generated once when the agent is first registered and preserved across
    /// all subsequent status/title updates. Unlike `pane_key.pane_id`, this ID
    /// is globally unique — pane IDs can be recycled by the multiplexer or
    /// shared across different tmux servers. Commands that need to address a
    /// specific agent (checkpoint, resume, focus) should use this ID.
    ///
    /// `skip_serializing_if` is intentionally absent: every state file must
    /// carry this field. The `default` allows old files (written before this
    /// field existed) to deserialise cleanly; they will receive a new UUID on
    /// the next write.
    #[serde(default = "new_agent_id")]
    pub agent_id: String,

    /// Composite identifier for the pane
    pub pane_key: PaneKey,

    /// Working directory of the agent
    pub workdir: PathBuf,

    /// Current agent status (working, waiting, done)
    pub status: Option<AgentStatus>,

    /// Unix timestamp when status was last set
    pub status_ts: Option<u64>,

    /// Pane title (set by Claude Code to show session summary)
    pub pane_title: Option<String>,

    /// PID of the pane's shell process (for pane ID recycling detection).
    /// This is the shell PID, not the agent PID.
    pub pane_pid: u32,

    /// Foreground command when status was set (for agent exit detection).
    /// If this changes (e.g., "node" -> "zsh"), the agent has exited.
    pub command: String,

    /// Unix timestamp of last persisted state update (any RPC call that writes state).
    /// Updated on status changes, title changes, and repeated same-status updates.
    /// Used for staleness detection, recency sorting, and interruption resume detection.
    pub updated_ts: u64,

    /// Window/tab name where this agent is running.
    /// Stored here because some backends (Zellij) can't query unfocused panes.
    #[serde(default)]
    pub window_name: Option<String>,

    /// Session name where this agent is running.
    /// Stored here for consistency with window_name.
    #[serde(default)]
    pub session_name: Option<String>,

    /// Multiplexer server boot identifier (e.g., tmux start_time).
    /// Used to distinguish intentional pane closes from server crashes:
    /// if this doesn't match the current server's boot_id, the server restarted.
    #[serde(default)]
    pub boot_id: Option<String>,

    /// Cached agent identity (canonical profile name, e.g. "claude", "kiro-cli").
    ///
    /// Classified once by `crate::agent::identity::classify_agent_kind` from the
    /// foreground command and pane title. Cached because tmux reports an
    /// agent's `pane_current_command` as a version string ("2.1.118") or a
    /// generic interpreter ("node", "Python") that the stem-based profile
    /// resolver cannot identify; the title that disambiguates them drifts
    /// over time, so we lock in the first definitive answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_kind: Option<String>,

    /// Sandbox VM / container handle for this agent (e.g. microsandbox VM name,
    /// Docker container ID). Used by checkpoint/resume to locate the sandbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_id: Option<String>,

    /// Path to the most recent sandbox checkpoint/snapshot for this agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_path: Option<PathBuf>,

    /// Unix timestamp when the last checkpoint was taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint_ts: Option<u64>,

    /// ID of the pipeline node currently executing in this agent (if any).
    /// Written by the pipeline runner when a node starts; cleared when it ends.
    /// None for agents not managed by a pipeline (foreign agents).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline_node_id: Option<String>,

    /// Human-readable title of the pipeline node (matches PipelineNode::title).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline_node_title: Option<String>,

    /// Which agent runtime owns this agent's process. `None` means the local
    /// runtime — the historical and default case, so records written before
    /// runtimes existed read correctly.
    ///
    /// Distinct from `pane_key.backend`, which names the *multiplexer*
    /// (tmux/wezterm). A remote-managed agent has a runtime and no multiplexer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,

    /// Agent-authored completion claim for the current instruction cycle.
    ///
    /// Distinct from `status`: `status` is a hook-driven turn-boundary signal
    /// that flips on every Stop hook, while `completion` is an explicit
    /// `workmux signal done|error` call meaning "the task is finished", not
    /// "a turn ended". Cleared on launch and on `workmux send` (a new
    /// instruction starts a new completion cycle).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<Completion>,
}

/// Outcome of an agent-authored completion signal (`workmux signal done|error`).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Completion {
    pub kind: CompletionKind,
    pub feedback: Option<String>,
    pub ts: u64,
}

/// Which outcome an agent claimed via `workmux signal done|error`.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CompletionKind {
    Completed,
    Failed,
}

impl AgentState {
    /// Convert to AgentPane for dashboard display.
    ///
    /// The caller is responsible for providing the best available session/window names
    /// (from live pane info when available, falling back to stored values).
    pub fn to_agent_pane(&self, session: String, window_name: String) -> AgentPane {
        AgentPane {
            session,
            window_name,
            pane_id: self.pane_key.pane_id.clone(),
            window_id: String::new(),
            path: self.workdir.clone(),
            pane_title: self.pane_title.clone(),
            status: self.status,
            status_ts: self.status_ts,
            updated_ts: Some(self.updated_ts),
            window_cmd: None,
            agent_command: Some(self.command.clone()),
            agent_kind: self.agent_kind.clone(),
            pipeline_node_title: self.pipeline_node_title.clone(),
            pane_pid: self.pane_pid,
            runtime: None,
        }
    }
}

/// Dashboard preferences stored globally.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct GlobalSettings {
    /// Sort mode: "priority", "project", "recency", "natural"
    pub sort_mode: String,

    /// Whether to hide stale agents in dashboard
    pub hide_stale: bool,

    /// Preview pane size percentage (10-90)
    pub preview_size: Option<u8>,

    /// Last visited agent pane_id (for quick toggle)
    pub last_pane_id: Option<String>,

    /// Dashboard scope filter: "all", "session", "project"
    #[serde(default)]
    pub dashboard_scope: Option<String>,

    /// Worktree sort mode: "natural", "age", "name", "project"
    #[serde(default)]
    pub worktree_sort_mode: Option<String>,

    /// Cycle state for the last-done command
    #[serde(default)]
    pub last_done_cycle: Option<LastDoneCycleState>,

    /// Sidebar layout mode: "compact" or "tiles"
    #[serde(default)]
    pub sidebar_layout: Option<String>,

    /// Sidebar width in columns (manual override synced across windows)
    #[serde(default)]
    pub sidebar_width: Option<u16>,

    /// Sidebar height in rows (manual override synced across windows)
    #[serde(default)]
    pub sidebar_height: Option<u16>,
}

/// Tracks which pane last-done navigated to, so repeated presses cycle
/// through the list instead of always jumping to index 0.
///
/// The cycle resets when a new agent appears at the top of the sorted list
/// (detected by `head_ts` changing).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct LastDoneCycleState {
    /// The pane that last-done most recently switched to.
    pub target: PaneKey,
    /// status_ts of the most recent done/waiting agent when the cycle started.
    /// If this changes, a new agent has finished and the cycle resets.
    pub head_ts: Option<u64>,
}

/// Ephemeral runtime state produced by the sidebar daemon.
///
/// Persisted to `runtime/<backend>__<instance>.json` so that the dashboard
/// (a separate process) can read daemon-derived signals without the daemon
/// writing to per-agent state files.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct RuntimeState {
    /// Pane IDs of agents detected as interrupted (working but no pane output change).
    #[serde(default)]
    pub interrupted_pane_ids: std::collections::HashSet<String>,

    /// Unix timestamp when this file was last written.
    /// Consumers should ignore the file if this is too old (daemon not running).
    pub updated_ts: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pane_key_to_filename() {
        let key = PaneKey {
            backend: "tmux".to_string(),
            instance: "default".to_string(),
            pane_id: "%1".to_string(),
        };
        // % is encoded as %25 for filesystem safety
        assert_eq!(key.to_filename(), "tmux__default__%251.json");
    }

    #[test]
    fn test_pane_key_from_filename() {
        // %25 decodes to %
        let key = PaneKey::from_filename("tmux__default__%251.json").unwrap();
        assert_eq!(key.backend, "tmux");
        assert_eq!(key.instance, "default");
        assert_eq!(key.pane_id, "%1");
    }

    #[test]
    fn test_pane_key_roundtrip() {
        let original = PaneKey {
            backend: "wezterm".to_string(),
            instance: "mux-123".to_string(),
            pane_id: "tab_5".to_string(),
        };
        let filename = original.to_filename();
        let parsed = PaneKey::from_filename(&filename).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn test_pane_key_from_invalid_filename() {
        assert!(PaneKey::from_filename("invalid.json").is_none());
        assert!(PaneKey::from_filename("only__two.json").is_none());
        assert!(PaneKey::from_filename("no_extension").is_none());
    }

    #[test]
    fn test_pane_key_with_underscores_in_pane_id() {
        let key = PaneKey {
            backend: "tmux".to_string(),
            instance: "default".to_string(),
            pane_id: "pane_with_underscores".to_string(),
        };
        let filename = key.to_filename();
        let parsed = PaneKey::from_filename(&filename).unwrap();
        assert_eq!(parsed.pane_id, "pane_with_underscores");
    }

    #[test]
    fn test_pane_key_with_socket_path() {
        // Real-world tmux socket path
        let key = PaneKey {
            backend: "tmux".to_string(),
            instance: "/private/tmp/tmux-501/default".to_string(),
            pane_id: "%79".to_string(),
        };
        let filename = key.to_filename();
        // Verify filename is safe (no slashes)
        assert!(!filename.contains('/'));
        // Verify roundtrip works
        let parsed = PaneKey::from_filename(&filename).unwrap();
        assert_eq!(parsed.instance, "/private/tmp/tmux-501/default");
        assert_eq!(parsed.pane_id, "%79");
    }

    fn sample_agent_state(completion: Option<Completion>) -> AgentState {
        AgentState {
            agent_id: "test-agent".to_string(),
            pane_key: PaneKey {
                backend: "tmux".to_string(),
                instance: "default".to_string(),
                pane_id: "%1".to_string(),
            },
            workdir: PathBuf::from("/tmp"),
            status: None,
            status_ts: None,
            pane_title: None,
            pane_pid: 1,
            command: "node".to_string(),
            updated_ts: 1234567890,
            window_name: None,
            session_name: None,
            boot_id: None,
            agent_kind: None,
            sandbox_id: None,
            checkpoint_path: None,
            checkpoint_ts: None,
            pipeline_node_id: None,
            pipeline_node_title: None,
            runtime: None,
            completion,
        }
    }

    #[test]
    fn test_agent_state_completion_roundtrip_some() {
        let original = sample_agent_state(Some(Completion {
            kind: CompletionKind::Completed,
            feedback: Some("all tests pass".to_string()),
            ts: 42,
        }));
        let json = serde_json::to_string(&original).unwrap();
        let parsed: AgentState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.completion, original.completion);
    }

    #[test]
    fn test_agent_state_completion_roundtrip_none() {
        let original = sample_agent_state(None);
        let json = serde_json::to_string(&original).unwrap();
        // completion is skip_serializing_if none: must not appear in the JSON.
        assert!(!json.contains("completion"));
        let parsed: AgentState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.completion, None);
    }

    #[test]
    fn test_agent_state_old_json_without_completion_field_deserialises() {
        // Simulates a state file written before `completion` existed.
        let json = r#"{
            "agent_id": "old-agent",
            "pane_key": {"backend": "tmux", "instance": "default", "pane_id": "%1"},
            "workdir": "/tmp",
            "status": null,
            "status_ts": null,
            "pane_title": null,
            "pane_pid": 1,
            "command": "node",
            "updated_ts": 100
        }"#;
        let parsed: AgentState = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.completion, None);
    }
}
