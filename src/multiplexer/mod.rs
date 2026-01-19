//! Multiplexer abstraction layer for terminal multiplexer backends.
//!
//! This module provides a trait-based abstraction that allows workmux to work
//! with different terminal multiplexers (tmux, WezTerm) interchangeably.

pub mod agent;
pub mod handshake;
pub mod tmux;
pub mod types;
pub mod util;

use anyhow::Result;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub use handshake::PaneHandshake;
pub use tmux::TmuxBackend;
pub use types::*;

use crate::config::{Config, PaneConfig};

/// Main trait for terminal multiplexer backends.
///
/// Implementations must be Send + Sync to allow sharing via Arc<dyn Multiplexer>.
pub trait Multiplexer: Send + Sync {
    /// Returns the name of this backend (e.g., "tmux", "wezterm")
    fn name(&self) -> &'static str;

    // === Server/Session ===

    /// Check if the multiplexer server is running
    fn is_running(&self) -> Result<bool>;

    /// Get the current pane ID from environment (TMUX_PANE or WEZTERM_PANE)
    fn current_pane_id(&self) -> Option<String>;

    /// Get the working directory of the active pane in the current client's session
    fn get_client_active_pane_path(&self) -> Result<PathBuf>;

    // === Window/Tab Management ===

    /// Create a new window/tab with the given parameters
    fn create_window(&self, params: CreateWindowParams) -> Result<String>;

    /// Kill a window by its full name (including prefix)
    fn kill_window(&self, full_name: &str) -> Result<()>;

    /// Schedule a window to close after a delay
    fn schedule_window_close(&self, full_name: &str, delay: Duration) -> Result<()>;

    /// Select (focus) a window by prefix and name
    fn select_window(&self, prefix: &str, name: &str) -> Result<()>;

    /// Check if a window exists by prefix and name
    fn window_exists(&self, prefix: &str, name: &str) -> Result<bool>;

    /// Check if a window exists by its full name
    fn window_exists_by_full_name(&self, full_name: &str) -> Result<bool>;

    /// Get the current window name, if running inside the multiplexer
    fn current_window_name(&self) -> Result<Option<String>>;

    /// Get all window names in the current session
    fn get_all_window_names(&self) -> Result<HashSet<String>>;

    /// Filter a list of window names, returning only those that still exist
    fn filter_active_windows(&self, windows: &[String]) -> Result<Vec<String>>;

    /// Find the last window (by index) that starts with the given prefix
    fn find_last_window_with_prefix(&self, prefix: &str) -> Result<Option<String>>;

    /// Find the last window that belongs to a specific base handle group
    fn find_last_window_with_base_handle(
        &self,
        prefix: &str,
        base_handle: &str,
    ) -> Result<Option<String>>;

    /// Wait until all specified windows are closed
    fn wait_until_windows_closed(&self, full_window_names: &[String]) -> Result<()>;

    /// Navigate to target window and close source window after a delay.
    ///
    /// This operation is used during cleanup (merge/remove) when the user is inside
    /// the window being closed. The delay allows the UI to update before navigation.
    ///
    /// Backends may implement this atomically (tmux run-shell) or as separate
    /// operations (schedule close + navigate).
    fn navigate_and_close_window(
        &self,
        prefix: &str,
        target_name: &str,
        source_name: &str,
        delay: Duration,
        trash_path: Option<&Path>,
    ) -> Result<()>;

    // === Pane Management ===

    /// Select (focus) a pane by ID
    fn select_pane(&self, pane_id: &str) -> Result<()>;

    /// Switch to a pane (may also switch windows/tabs as needed)
    fn switch_to_pane(&self, pane_id: &str) -> Result<()>;

    /// Respawn a pane with optional command. Returns the (possibly new) pane ID.
    fn respawn_pane(&self, pane_id: &str, cwd: &Path, cmd: Option<&str>) -> Result<String>;

    /// Capture the content of a pane
    fn capture_pane(&self, pane_id: &str, lines: u16) -> Option<String>;

    // === Text I/O ===

    /// Send keys (command + Enter) to a pane
    fn send_keys(&self, pane_id: &str, command: &str) -> Result<()>;

    /// Send keys to an agent pane, with special handling for Claude's ! prefix
    fn send_keys_to_agent(&self, pane_id: &str, command: &str, agent: Option<&str>) -> Result<()>;

    /// Send a single key to a pane
    fn send_key(&self, pane_id: &str, key: &str) -> Result<()>;

    /// Paste multiline content to a pane (using bracketed paste)
    fn paste_multiline(&self, pane_id: &str, content: &str) -> Result<()>;

    // === Shell ===

    /// Get the default shell for new panes
    fn get_default_shell(&self) -> Result<String>;

    /// Create a handshake mechanism for synchronizing shell startup
    fn create_handshake(&self) -> Result<Box<dyn PaneHandshake>>;

    // === Status ===

    /// Set status icon for a pane, optionally enabling exit detection
    fn set_status(&self, pane_id: &str, icon: &str, exit_detection: bool) -> Result<()>;

    /// Clear status from a pane
    fn clear_status(&self, pane_id: &str) -> Result<()>;

    /// Ensure the status format is configured (for backends that need it)
    fn ensure_status_format(&self, pane_id: &str) -> Result<()>;

    // === Pane Setup ===

    /// Setup panes in a window according to configuration
    fn setup_panes(
        &self,
        initial_pane_id: &str,
        panes: &[PaneConfig],
        working_dir: &Path,
        options: PaneSetupOptions<'_>,
        config: &Config,
        task_agent: Option<&str>,
    ) -> Result<PaneSetupResult>;

    // === Multi-Session/Workspace Support ===

    /// Get all window names across ALL sessions/workspaces.
    ///
    /// Default implementation returns same as get_all_window_names() (single session).
    /// WezTerm overrides this to return windows from all workspaces.
    #[allow(dead_code)] // Reserved for future multi-session features
    fn get_all_window_names_all_sessions(&self) -> Result<HashSet<String>> {
        self.get_all_window_names()
    }

    // === State Reconciliation ===

    /// Get the backend instance identifier (socket path, mux ID, etc.).
    ///
    /// This is used to create unique state file paths when multiple instances
    /// of the same backend are running (e.g., multiple tmux servers).
    ///
    /// For tmux: socket path or "default" for standard socket
    /// For WezTerm: mux domain ID
    fn instance_id(&self) -> String;

    /// Get live pane info including PID and current command.
    ///
    /// Returns None if pane does not exist. Used during state reconciliation
    /// to validate stored state against actual pane state.
    fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>>;
}

/// Detect which backend to use based on environment and config.
///
/// Priority order:
/// 1. WORKMUX_BACKEND environment variable
/// 2. Config file backend setting
/// 3. Default to tmux
pub fn detect_backend(config: &Config) -> BackendType {
    // 1. Environment variable has highest priority
    if let Ok(env_backend) = std::env::var("WORKMUX_BACKEND") {
        match env_backend.to_lowercase().as_str() {
            "wezterm" => return BackendType::WezTerm,
            "tmux" => return BackendType::Tmux,
            other => {
                eprintln!(
                    "workmux: unknown backend '{}' in WORKMUX_BACKEND, falling back to tmux",
                    other
                );
                return BackendType::Tmux;
            }
        }
    }

    // 2. Config file backend setting
    if let Some(backend) = &config.backend {
        match backend.to_lowercase().as_str() {
            "wezterm" => return BackendType::WezTerm,
            "tmux" => return BackendType::Tmux,
            other => {
                eprintln!(
                    "workmux: unknown backend '{}' in config, falling back to tmux",
                    other
                );
                return BackendType::Tmux;
            }
        }
    }

    // 3. Default to tmux for backward compatibility
    BackendType::Tmux
}

/// Create a backend instance based on the backend type.
pub fn create_backend(backend_type: BackendType) -> Arc<dyn Multiplexer> {
    match backend_type {
        BackendType::Tmux => Arc::new(TmuxBackend::new()),
        BackendType::WezTerm => {
            // WezTerm backend not yet implemented
            panic!("WezTerm backend is not yet available")
        }
    }
}
