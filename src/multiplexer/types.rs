//! Shared types for multiplexer backends.
//!
//! These types are used by both the tmux and WezTerm backends.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Agent status representing the current state of an agent.
///
/// Stored as lowercase strings in JSON (e.g., "working", "waiting", "done").
/// Icons are resolved at display time from config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    /// Agent is actively processing
    Working,
    /// Agent needs user input
    Waiting,
    /// Agent has finished
    Done,
}

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
    /// Current agent status
    pub status: Option<AgentStatus>,
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

/// Parameters for creating a new session
#[derive(Debug, Clone)]
pub struct CreateSessionParams<'a> {
    /// Prefix for the session name (e.g., "wm-")
    pub prefix: &'a str,
    /// Base session name
    pub name: &'a str,
    /// Working directory for the session's initial window
    pub cwd: &'a std::path::Path,
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
    /// Root of the worktree (for sandbox mounting). May differ from working_dir in monorepos.
    pub worktree_root: Option<&'a std::path::Path>,
    /// Pre-booted Lima VM name (if sandbox backend is Lima and VM was booted before window creation)
    pub lima_vm_name: Option<&'a str>,
}

/// Backend type for multiplexer selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendType {
    /// tmux backend (default)
    #[default]
    Tmux,
    /// WezTerm backend
    WezTerm,
    /// Kitty backend
    Kitty,
}

impl std::fmt::Display for BackendType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackendType::Tmux => write!(f, "tmux"),
            BackendType::WezTerm => write!(f, "wezterm"),
            BackendType::Kitty => write!(f, "kitty"),
        }
    }
}

impl std::str::FromStr for BackendType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tmux" => Ok(BackendType::Tmux),
            "wezterm" => Ok(BackendType::WezTerm),
            "kitty" => Ok(BackendType::Kitty),
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
