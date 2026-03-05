//! Scope filter for the dashboard agent list.

use crate::state::StateStore;

/// Whether the dashboard shows all agents or only those in the current session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeMode {
    /// Show all agents across all sessions
    #[default]
    All,
    /// Show only agents in the current session
    Session,
}

impl ScopeMode {
    /// Toggle between All and Session
    pub fn toggle(self) -> Self {
        match self {
            ScopeMode::All => ScopeMode::Session,
            ScopeMode::Session => ScopeMode::All,
        }
    }

    /// Get the display label for the scope mode
    pub fn label(&self) -> &'static str {
        match self {
            ScopeMode::All => "all",
            ScopeMode::Session => "session",
        }
    }

    /// Convert to string for storage.
    fn as_str(&self) -> &'static str {
        match self {
            ScopeMode::All => "all",
            ScopeMode::Session => "session",
        }
    }

    /// Parse from storage string.
    fn from_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "session" => ScopeMode::Session,
            _ => ScopeMode::All,
        }
    }

    /// Load scope mode from StateStore.
    pub fn load() -> Self {
        StateStore::new()
            .ok()
            .and_then(|store| store.load_settings().ok())
            .and_then(|s| s.dashboard_scope)
            .map(|s| Self::from_str(&s))
            .unwrap_or_default()
    }

    /// Save scope mode to StateStore.
    pub fn save(&self) {
        if let Ok(store) = StateStore::new()
            && let Ok(mut settings) = store.load_settings()
        {
            settings.dashboard_scope = Some(self.as_str().to_string());
            let _ = store.save_settings(&settings);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_toggle() {
        assert_eq!(ScopeMode::All.toggle(), ScopeMode::Session);
        assert_eq!(ScopeMode::Session.toggle(), ScopeMode::All);
    }

    #[test]
    fn test_labels() {
        assert_eq!(ScopeMode::All.label(), "all");
        assert_eq!(ScopeMode::Session.label(), "session");
    }

    #[test]
    fn test_roundtrip_strings() {
        for mode in [ScopeMode::All, ScopeMode::Session] {
            assert_eq!(ScopeMode::from_str(mode.as_str()), mode);
        }
    }

    #[test]
    fn test_from_str_defaults_to_all() {
        assert_eq!(ScopeMode::from_str(""), ScopeMode::All);
        assert_eq!(ScopeMode::from_str("unknown"), ScopeMode::All);
        assert_eq!(ScopeMode::from_str("project"), ScopeMode::All);
    }

    #[test]
    fn test_from_str_case_insensitive() {
        assert_eq!(ScopeMode::from_str("Session"), ScopeMode::Session);
        assert_eq!(ScopeMode::from_str("SESSION"), ScopeMode::Session);
    }
}
