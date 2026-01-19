//! Core data structures for filesystem-based state storage.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::multiplexer::types::AgentPane;

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

    /// Pane identifier within the backend
    pub pane_id: String,
}

impl PaneKey {
    /// Generate filename for this pane's state file.
    ///
    /// Format: `{backend}__{instance}__{pane_id}.json`
    /// Double underscores used since pane IDs may contain single underscores.
    pub fn to_filename(&self) -> String {
        format!("{}__{}__{}.json", self.backend, self.instance, self.pane_id)
    }

    /// Parse a PaneKey from a filename.
    ///
    /// Returns None if the filename doesn't match the expected format.
    pub fn from_filename(filename: &str) -> Option<Self> {
        let stem = filename.strip_suffix(".json")?;
        let parts: Vec<&str> = stem.splitn(3, "__").collect();
        if parts.len() == 3 {
            Some(PaneKey {
                backend: parts[0].to_string(),
                instance: parts[1].to_string(),
                pane_id: parts[2].to_string(),
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
    /// Composite identifier for the pane
    pub pane_key: PaneKey,

    /// Working directory of the agent
    pub workdir: PathBuf,

    /// Current status icon (e.g., "🤖", "💬", "✅")
    pub status: Option<String>,

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

    /// Unix timestamp of last state update (status change).
    /// Note: This is NOT a heartbeat - only updated when status changes.
    /// Used for staleness detection and recency sorting.
    pub updated_ts: u64,
}

impl AgentState {
    /// Convert to AgentPane for dashboard display.
    ///
    /// Requires session/window info from multiplexer since we don't store those.
    pub fn to_agent_pane(&self, session: String, window_name: String) -> AgentPane {
        AgentPane {
            session,
            window_name,
            pane_id: self.pane_key.pane_id.clone(),
            path: self.workdir.clone(),
            pane_title: self.pane_title.clone(),
            status: self.status.clone(),
            status_ts: self.status_ts,
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
        assert_eq!(key.to_filename(), "tmux__default__%1.json");
    }

    #[test]
    fn test_pane_key_from_filename() {
        let key = PaneKey::from_filename("tmux__default__%1.json").unwrap();
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
}
