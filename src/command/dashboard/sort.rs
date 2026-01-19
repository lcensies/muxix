//! Sort mode logic for the dashboard agent list.

use crate::state::StateStore;

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

    /// Convert to string for storage.
    fn as_str(&self) -> &'static str {
        match self {
            SortMode::Priority => "priority",
            SortMode::Project => "project",
            SortMode::Recency => "recency",
            SortMode::Natural => "natural",
        }
    }

    /// Parse from storage string.
    fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "project" => SortMode::Project,
            "recency" => SortMode::Recency,
            "natural" => SortMode::Natural,
            _ => SortMode::Priority, // Default fallback
        }
    }

    /// Load sort mode from StateStore.
    pub fn load() -> Self {
        StateStore::new()
            .ok()
            .and_then(|store| store.load_settings().ok())
            .map(|s| Self::from_str(&s.sort_mode))
            .unwrap_or_default()
    }

    /// Save sort mode to StateStore.
    pub fn save(&self) {
        if let Ok(store) = StateStore::new()
            && let Ok(mut settings) = store.load_settings() {
                settings.sort_mode = self.as_str().to_string();
                let _ = store.save_settings(&settings);
            }
    }
}
