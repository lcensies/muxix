//! Sort mode logic for the dashboard agent list.

use crate::cmd::Cmd;

const TMUX_SORT_MODE_VAR: &str = "@workmux_sort_mode";

/// Available sort modes for the agent list
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortMode {
    /// Sort by agent status importance (Waiting > Done > Working > Stale)
    #[default]
    Priority,
    /// Group agents by project name, then by status within each project
    Project,
    /// Sort by duration since last status change (newest first)
    Recency,
    /// Natural tmux order (by pane_id)
    Natural,
}

impl SortMode {
    /// Cycle to the next sort mode
    pub fn next(self) -> Self {
        match self {
            SortMode::Priority => SortMode::Project,
            SortMode::Project => SortMode::Recency,
            SortMode::Recency => SortMode::Natural,
            SortMode::Natural => SortMode::Priority,
        }
    }

    /// Get the display name for the sort mode
    pub fn label(&self) -> &'static str {
        match self {
            SortMode::Priority => "Priority",
            SortMode::Project => "Project",
            SortMode::Recency => "Recency",
            SortMode::Natural => "Natural",
        }
    }

    /// Convert to string for tmux storage
    fn as_str(&self) -> &'static str {
        match self {
            SortMode::Priority => "priority",
            SortMode::Project => "project",
            SortMode::Recency => "recency",
            SortMode::Natural => "natural",
        }
    }

    /// Parse from tmux storage string
    fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "project" => SortMode::Project,
            "recency" => SortMode::Recency,
            "natural" => SortMode::Natural,
            _ => SortMode::Priority, // Default fallback
        }
    }

    /// Load sort mode from tmux global variable
    pub fn load_from_tmux() -> Self {
        Cmd::new("tmux")
            .args(&["show-option", "-gqv", TMUX_SORT_MODE_VAR])
            .run_and_capture_stdout()
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| Self::from_str(&s))
            .unwrap_or_default()
    }

    /// Save sort mode to tmux global variable
    pub fn save_to_tmux(&self) {
        let _ = Cmd::new("tmux")
            .args(&["set-option", "-g", TMUX_SORT_MODE_VAR, self.as_str()])
            .run();
    }
}
