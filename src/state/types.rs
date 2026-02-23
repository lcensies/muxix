//! Core data structures for filesystem-based state storage.

use percent_encoding::{AsciiSet, CONTROLS, percent_decode_str, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Characters that need encoding in filenames (beyond control chars).
/// Includes path separators and other filesystem-unsafe characters.
const FILENAME_ENCODE_SET: &AsciiSet = &CONTROLS.add(b'/').add(b'\\').add(b':').add(b'%');

use crate::multiplexer::types::{AgentPane, AgentStatus};

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

    /// Unix timestamp of last state update (status change).
    /// Note: This is NOT a heartbeat - only updated when status changes.
    /// Used for staleness detection and recency sorting.
    pub updated_ts: u64,

    /// Window/tab name where this agent is running.
    /// Stored here because some backends (Zellij) can't query unfocused panes.
    #[serde(default)]
    pub window_name: Option<String>,

    /// Session name where this agent is running.
    /// Stored here for consistency with window_name.
    #[serde(default)]
    pub session_name: Option<String>,

    /// Unix timestamp of last heartbeat (dashboard refresh).
    /// Used by Zellij to detect dead agents faster than staleness timeout.
    /// Backends with reliable PID validation (tmux/WezTerm) don't need this.
    #[serde(default)]
    pub last_heartbeat: Option<u64>,
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
            path: self.workdir.clone(),
            pane_title: self.pane_title.clone(),
            status: self.status,
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

    /// Last visited agent pane_id (for quick toggle)
    pub last_pane_id: Option<String>,
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
}
