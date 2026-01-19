//! Shared types for multiplexer backends.
//!
//! These types are used by both the tmux and WezTerm backends.

use std::path::PathBuf;

/// Information about a specific pane running a workmux agent
#[derive(Debug, Clone)]
pub struct AgentPane {
    /// Session name (tmux session or WezTerm workspace)
    pub session: String,
    /// Window name (e.g., wm-feature-auth)
    pub window_name: String,
    /// Pane ID (e.g., %0 for tmux, numeric for WezTerm)
    pub pane_id: String,
    /// Working directory path of the pane
    pub path: PathBuf,
    /// Pane title (set by Claude Code to show session summary)
    pub pane_title: Option<String>,
    /// Current status icon (if set)
    pub status: Option<String>,
    /// Unix timestamp when status was last set
    pub status_ts: Option<u64>,
}

/// Parameters for creating a new window/tab
#[derive(Debug, Clone)]
pub struct CreateWindowParams<'a> {
    /// Prefix for the window name (e.g., "wm-")
    pub prefix: &'a str,
    /// Base window name
    pub name: &'a str,
    /// Working directory for the window
    pub cwd: &'a std::path::Path,
    /// Optional window ID to insert after (for ordering)
    pub after_window: Option<&'a str>,
}

/// Result of setting up panes in a window
#[derive(Debug, Clone)]
pub struct PaneSetupResult {
    /// The ID of the pane that should receive focus
    pub focus_pane_id: String,
}

/// Options for pane setup
#[derive(Debug, Clone)]
pub struct PaneSetupOptions<'a> {
    /// Whether to run commands in the panes
    pub run_commands: bool,
    /// Path to the prompt file for agent panes
    pub prompt_file_path: Option<&'a std::path::Path>,
}

/// Backend type for multiplexer selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendType {
    /// tmux backend (default)
    #[default]
    Tmux,
    /// WezTerm backend
    WezTerm,
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendType::Tmux => write!(f, "tmux"),
            BackendType::WezTerm => write!(f, "wezterm"),
        }
    }
}

impl std::str::FromStr for BackendType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tmux" => Ok(BackendType::Tmux),
            "wezterm" => Ok(BackendType::WezTerm),
            other => Err(format!("unknown backend: {}", other)),
        }
    }
}

/// Live pane information from the multiplexer (used for reconciliation).
///
/// Contains current state of a pane as queried from the multiplexer,
/// used to validate stored state against actual pane state.
#[derive(Debug, Clone)]
pub struct LivePaneInfo {
    /// PID of the pane's shell process
    pub pid: u32,

    /// Current foreground command (e.g., "node", "zsh")
    pub current_command: String,

    /// Working directory
    pub working_dir: PathBuf,

    /// Pane title (if set)
    pub title: Option<String>,

    /// Session name (tmux session or WezTerm workspace)
    pub session: Option<String>,

    /// Window name
    pub window: Option<String>,
}
