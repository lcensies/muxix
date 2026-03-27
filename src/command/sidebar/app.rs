//! Application state for the sidebar TUI.

use anyhow::Result;
use ratatui::widgets::ListState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::cmd::Cmd;
use crate::command::dashboard::agent::{extract_project_name, extract_worktree_name};
use crate::config::{Config, StatusIcons};

use crate::multiplexer::{AgentPane, Multiplexer};

use crate::command::dashboard::ui::theme::ThemePalette;

use super::snapshot::SidebarSnapshot;

/// Sidebar layout mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarLayoutMode {
    #[default]
    Compact,
    Tiles,
}

impl SidebarLayoutMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Tiles => "tiles",
        }
    }
}

/// Whether the sidebar auto-follows its host window or the user is navigating manually.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionMode {
    FollowHost,
    Manual,
}

/// Lightweight sidebar app state. No preview, git, PR, diff, or input mode.
pub struct SidebarApp {
    pub mux: Arc<dyn Multiplexer>,
    pub agents: Vec<AgentPane>,
    pub list_state: ListState,
    pub should_quit: bool,
    pub palette: ThemePalette,
    pub status_icons: StatusIcons,
    pub spinner_frame: u8,
    pub stale_threshold_secs: u64,
    pub layout_mode: SidebarLayoutMode,
    /// Window prefix from config
    window_prefix: String,
    /// The sidebar's own host session (immutable, detected once at startup via TMUX_PANE)
    host_session: Option<String>,
    /// Stable tmux window ID (e.g., @42) for active-window detection
    host_window_id: Option<String>,
    /// Index of the agent in the sidebar's host window (updated each snapshot)
    pub host_agent_idx: Option<usize>,
    /// Whether this sidebar's host window is the active window in the session
    host_window_active: bool,
    selection_mode: SelectionMode,
}

impl SidebarApp {
    /// Create a new sidebar client. Does config + host detection only, no tmux polling.
    pub fn new_client(mux: Arc<dyn Multiplexer>) -> Result<Self> {
        let config = Config::load(None)?;

        let theme_mode = config
            .theme
            .mode
            .unwrap_or_else(|| match terminal_light::luma() {
                Ok(luma) if luma > 0.6 => crate::config::ThemeMode::Light,
                _ => crate::config::ThemeMode::Dark,
            });
        let palette = ThemePalette::for_scheme(config.theme.scheme, theme_mode);
        let window_prefix = config.window_prefix().to_string();
        let status_icons = config.status_icons.clone();

        let (host_session, _host_window, host_window_id) = detect_host_window();

        Ok(Self {
            mux,
            agents: Vec::new(),
            list_state: ListState::default(),
            should_quit: false,
            palette,
            status_icons,
            spinner_frame: 0,
            stale_threshold_secs: 60 * 60, // 60 minutes
            layout_mode: SidebarLayoutMode::default(),
            window_prefix,
            host_session,
            host_window_id,
            host_agent_idx: None,
            host_window_active: true,
            selection_mode: SelectionMode::FollowHost,
        })
    }

    /// Apply a snapshot received from the daemon.
    pub fn apply_snapshot(&mut self, snapshot: &SidebarSnapshot) {
        self.layout_mode = snapshot.layout_mode;

        // Find host agent by window_id (stable tmux ID, survives renames)
        self.host_agent_idx = self
            .host_window_id
            .as_ref()
            .and_then(|wid| snapshot.agents.iter().position(|a| a.window_id == *wid));

        let agents: Vec<AgentPane> = snapshot.agents.iter().map(|a| a.to_agent_pane()).collect();

        // Check if host window is active
        let was_active = self.host_window_active;
        self.host_window_active =
            if let (Some(session), Some(window_id)) = (&self.host_session, &self.host_window_id) {
                snapshot
                    .active_windows
                    .contains(&(session.clone(), window_id.clone()))
            } else {
                true
            };

        // Re-arm FollowHost when window becomes active
        if !was_active && self.host_window_active {
            self.selection_mode = SelectionMode::FollowHost;
        }

        // Preserve selection by pane_id
        let selected_pane = self
            .list_state
            .selected()
            .and_then(|i| self.agents.get(i))
            .map(|a| a.pane_id.clone());

        self.agents = agents;

        // Restore selection
        if let Some(ref pane_id) = selected_pane {
            if let Some(idx) = self.agents.iter().position(|a| &a.pane_id == pane_id) {
                self.list_state.select(Some(idx));
            } else if !self.agents.is_empty() {
                let clamped = self
                    .list_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.agents.len() - 1);
                self.list_state.select(Some(clamped));
            } else {
                self.list_state.select(None);
            }
        } else if !self.agents.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }

        self.sync_selection();
    }

    /// Select the agent belonging to this sidebar's host window (only in FollowHost mode).
    pub fn sync_selection(&mut self) {
        if self.selection_mode != SelectionMode::FollowHost {
            return;
        }
        if let Some(idx) = self.host_agent_idx {
            self.list_state.select(Some(idx));
        }
    }

    pub fn tick(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1) % 10;
    }

    pub fn next(&mut self) {
        self.selection_mode = SelectionMode::Manual;
        if self.agents.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let next = if i >= self.agents.len() - 1 { 0 } else { i + 1 };
        self.list_state.select(Some(next));
    }

    pub fn previous(&mut self) {
        self.selection_mode = SelectionMode::Manual;
        if self.agents.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let prev = if i == 0 { self.agents.len() - 1 } else { i - 1 };
        self.list_state.select(Some(prev));
    }

    pub fn select_first(&mut self) {
        self.selection_mode = SelectionMode::Manual;
        if !self.agents.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub fn select_last(&mut self) {
        self.selection_mode = SelectionMode::Manual;
        if !self.agents.is_empty() {
            self.list_state.select(Some(self.agents.len() - 1));
        }
    }

    pub fn jump_to_selected(&mut self) {
        if let Some(idx) = self.list_state.selected()
            && let Some(agent) = self.agents.get(idx)
        {
            let pane_id = agent.pane_id.clone();
            let _ = self.mux.switch_to_pane(&pane_id, None);
        }
    }

    pub fn toggle_layout_mode(&mut self) {
        self.layout_mode = match self.layout_mode {
            SidebarLayoutMode::Compact => SidebarLayoutMode::Tiles,
            SidebarLayoutMode::Tiles => SidebarLayoutMode::Compact,
        };
        // Persist to tmux so all sidebar instances pick it up
        let _ = Cmd::new("tmux")
            .args(&[
                "set-option",
                "-g",
                "@workmux_sidebar_layout",
                self.layout_mode.as_str(),
            ])
            .run();
    }

    pub fn window_prefix(&self) -> &str {
        &self.window_prefix
    }

    /// Display name for an agent: "project/worktree" or just "project" for main.
    pub fn display_name(&self, agent: &AgentPane) -> String {
        let project = extract_project_name(&agent.path);
        let (worktree, is_main) =
            extract_worktree_name(&agent.session, &agent.window_name, &self.window_prefix);

        if is_main {
            project
        } else {
            format!("{}/{}", project, worktree)
        }
    }
}

/// Detect this sidebar's host window using TMUX_PANE (stable, one-time).
/// Returns (session, window_name, window_id).
fn detect_host_window() -> (Option<String>, Option<String>, Option<String>) {
    let pane_id = std::env::var("TMUX_PANE").ok().unwrap_or_default();
    let target = if pane_id.is_empty() {
        None
    } else {
        Some(pane_id)
    };
    let mut args = vec!["display-message", "-p"];
    if let Some(ref t) = target {
        args.extend_from_slice(&["-t", t]);
    }
    args.push("#{session_name}\t#{window_name}\t#{window_id}");
    let output = Cmd::new("tmux")
        .args(&args)
        .run_and_capture_stdout()
        .ok()
        .unwrap_or_default();
    let trimmed = output.trim();
    let parts: Vec<&str> = trimmed.split('\t').collect();
    if parts.len() >= 3 {
        let s = (!parts[0].is_empty()).then(|| parts[0].to_string());
        let w = (!parts[1].is_empty()).then(|| parts[1].to_string());
        let id = (!parts[2].is_empty()).then(|| parts[2].to_string());
        (s, w, id)
    } else if parts.len() == 2 {
        let s = (!parts[0].is_empty()).then(|| parts[0].to_string());
        let w = (!parts[1].is_empty()).then(|| parts[1].to_string());
        (s, w, None)
    } else {
        (None, None, None)
    }
}
