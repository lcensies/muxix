//! Application state for the sidebar TUI.

use anyhow::Result;
use ratatui::widgets::ListState;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{Config, StatusIcons};
use crate::multiplexer::{AgentPane, AgentStatus, Multiplexer};
use crate::state::StateStore;

use crate::command::dashboard::ui::theme::ThemePalette;

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
    /// Session name at launch time (for session scope filtering)
    launch_session: Option<String>,
    /// Window prefix from config
    window_prefix: String,
}

impl SidebarApp {
    pub fn new(mux: Arc<dyn Multiplexer>) -> Result<Self> {
        let config = Config::load(None)?;

        let theme_mode = config
            .theme
            .mode
            .unwrap_or_else(|| match terminal_light::luma() {
                Ok(luma) if luma > 0.6 => crate::config::ThemeMode::Light,
                _ => crate::config::ThemeMode::Dark,
            });
        let palette = ThemePalette::for_scheme(config.theme.scheme, theme_mode);
        let launch_session = mux.current_session();
        let window_prefix = config.window_prefix().to_string();
        let status_icons = config.status_icons.clone();

        let mut app = Self {
            mux,
            agents: Vec::new(),
            list_state: ListState::default(),
            should_quit: false,
            palette,
            status_icons,
            spinner_frame: 0,
            stale_threshold_secs: 60 * 60, // 60 minutes
            launch_session,
            window_prefix,
        };

        app.refresh();

        if !app.agents.is_empty() {
            app.list_state.select(Some(0));
        }

        Ok(app)
    }

    pub fn refresh(&mut self) {
        let mut agents = StateStore::new()
            .and_then(|store| store.load_reconciled_agents(self.mux.as_ref()))
            .unwrap_or_default();

        // Filter to current session
        if let Some(ref session) = self.launch_session {
            agents.retain(|a| a.session == *session);
        }

        // Sort by priority (same logic as dashboard)
        let stale_threshold = self.stale_threshold_secs;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        agents.sort_by_cached_key(|a| {
            let is_stale = a
                .status_ts
                .map(|ts| now.saturating_sub(ts) > stale_threshold)
                .unwrap_or(false);
            let priority = if is_stale {
                3u8
            } else {
                match a.status {
                    Some(AgentStatus::Waiting) => 0,
                    Some(AgentStatus::Done) => 1,
                    Some(AgentStatus::Working) => 2,
                    None => 3,
                }
            };
            let elapsed = a
                .status_ts
                .map(|ts| now.saturating_sub(ts))
                .unwrap_or(u64::MAX);
            let pane_num: u64 = a
                .pane_id
                .strip_prefix('%')
                .unwrap_or(&a.pane_id)
                .parse()
                .unwrap_or(u64::MAX);
            (priority, elapsed, pane_num)
        });

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
    }

    pub fn tick(&mut self) {
        self.spinner_frame = self.spinner_frame.wrapping_add(1) % 10;
    }

    pub fn next(&mut self) {
        if self.agents.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let next = if i >= self.agents.len() - 1 { 0 } else { i + 1 };
        self.list_state.select(Some(next));
    }

    pub fn previous(&mut self) {
        if self.agents.is_empty() {
            return;
        }
        let i = self.list_state.selected().unwrap_or(0);
        let prev = if i == 0 { self.agents.len() - 1 } else { i - 1 };
        self.list_state.select(Some(prev));
    }

    pub fn select_first(&mut self) {
        if !self.agents.is_empty() {
            self.list_state.select(Some(0));
        }
    }

    pub fn select_last(&mut self) {
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

    /// Extract worktree display name from an agent pane.
    pub fn worktree_name(&self, agent: &AgentPane) -> String {
        if let Some(stripped) = agent.window_name.strip_prefix(self.window_prefix.as_str()) {
            stripped.to_string()
        } else if let Some(stripped) = agent.session.strip_prefix(self.window_prefix.as_str()) {
            stripped.to_string()
        } else {
            "main".to_string()
        }
    }
}
