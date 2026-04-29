//! Application state for the sidebar TUI.

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::agent_display::{extract_project_name, extract_worktree_name, resolve_labels};
use crate::cmd::Cmd;
use crate::config::{AgentIcons, Config, StatusIcons};
use crate::git::GitStatus;

use crate::multiplexer::{AgentPane, Multiplexer};

use crate::ui::theme::ThemePalette;

use super::snapshot::SidebarSnapshot;
use super::template::parser::{Token, parse_line};

/// Sidebar layout mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SidebarLayoutMode {
    Compact,
    #[default]
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

const DEFAULT_COMPACT_TEMPLATE: &str = "{status_icon} {primary}{pane_suffix} {fill} {elapsed}";
const DEFAULT_TILE_TEMPLATES: &[&str] = &[
    "{primary}{pane_suffix} {fill} {elapsed}",
    "{secondary} {fill} {git_stats}",
    "{pane_title}",
];

/// Parsed templates for one sidebar instance.
#[derive(Debug, Clone)]
pub struct ParsedTemplates {
    pub compact: Vec<Token>,
    pub tiles: Vec<Vec<Token>>,
}

/// Lightweight sidebar app state. No preview, git, PR, diff, or input mode.
pub struct SidebarApp {
    pub mux: Arc<dyn Multiplexer>,
    pub agents: Vec<AgentPane>,
    pub list_state: ListState,
    pub should_quit: bool,
    pub quit_reason: Option<String>,
    pub palette: ThemePalette,
    pub status_icons: StatusIcons,
    pub spinner_frame: u8,
    pub stale_threshold_secs: u64,
    pub layout_mode: SidebarLayoutMode,
    /// Area where the list was last rendered (for mouse hit testing)
    pub list_area: Rect,
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
    /// Git status per worktree path (received from daemon snapshots).
    pub git_statuses: HashMap<PathBuf, GitStatus>,
    /// Pane IDs of agents detected as interrupted by the daemon.
    pub interrupted_pane_ids: std::collections::HashSet<String>,
    /// Pane IDs of agents manually marked as sleeping by the user.
    pub sleeping_pane_ids: std::collections::HashSet<String>,
    /// Parsed sidebar templates.
    pub templates: ParsedTemplates,
    /// Per-agent icon overrides.
    pub agent_icons: AgentIcons,
    /// Cached tile heights for hit testing (updated each render).
    pub tile_heights: Vec<usize>,
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
        let palette = ThemePalette::from_config(&config.theme, theme_mode);
        let window_prefix = config.window_prefix().to_string();
        let status_icons = config.status_icons.clone();

        let (host_session, host_window_id) = detect_host_window();

        let templates = parse_templates(&config);
        let agent_icons = config.sidebar.agent_icons.clone().unwrap_or_default();

        Ok(Self {
            mux,
            agents: Vec::new(),
            list_state: ListState::default(),
            should_quit: false,
            quit_reason: None,
            palette,
            status_icons,
            spinner_frame: 0,
            stale_threshold_secs: 60 * 60, // 60 minutes
            layout_mode: SidebarLayoutMode::default(),
            list_area: Rect::default(),
            window_prefix,
            host_session,
            host_window_id,
            host_agent_idx: None,
            host_window_active: true,
            selection_mode: SelectionMode::FollowHost,
            git_statuses: HashMap::new(),
            interrupted_pane_ids: std::collections::HashSet::new(),
            sleeping_pane_ids: std::collections::HashSet::new(),
            templates,
            agent_icons,
            tile_heights: Vec::new(),
        })
    }

    /// Apply a snapshot received from the daemon.
    pub fn apply_snapshot(&mut self, snapshot: SidebarSnapshot) {
        self.layout_mode = snapshot.layout_mode;
        self.git_statuses = snapshot.git_statuses;
        self.interrupted_pane_ids = snapshot.interrupted_pane_ids;
        self.sleeping_pane_ids = snapshot.sleeping_pane_ids;

        // Find host agent by window_id (stable tmux ID, survives renames).
        // When multiple agents share a window, prefer the active pane.
        self.host_agent_idx = self.host_window_id.as_ref().and_then(|wid| {
            let mut first_match = None;
            for (i, agent) in snapshot.agents.iter().enumerate() {
                if agent.window_id != *wid {
                    continue;
                }
                if snapshot.active_pane_ids.contains(&agent.pane_id) {
                    return Some(i);
                }
                first_match.get_or_insert(i);
            }
            first_match
        });

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

        self.agents = snapshot.agents;

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

    pub fn host_window_id(&self) -> Option<&str> {
        self.host_window_id.as_deref()
    }

    pub fn host_window_active(&self) -> bool {
        self.host_window_active
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

    pub fn select_index(&mut self, idx: usize) {
        self.selection_mode = SelectionMode::Manual;
        if !self.agents.is_empty() {
            self.list_state.select(Some(idx.min(self.agents.len() - 1)));
        }
    }

    pub fn scroll_up(&mut self) {
        self.selection_mode = SelectionMode::Manual;
        if let Some(i) = self.list_state.selected() {
            self.list_state.select(Some(i.saturating_sub(1)));
        }
    }

    pub fn scroll_down(&mut self) {
        self.selection_mode = SelectionMode::Manual;
        if let Some(i) = self.list_state.selected() {
            let last = self.agents.len().saturating_sub(1);
            self.list_state.select(Some((i + 1).min(last)));
        }
    }

    pub fn hit_test(&self, _column: u16, row: u16) -> Option<usize> {
        if self.agents.is_empty() {
            return None;
        }
        let area = self.list_area;
        if row < area.y || row >= area.y + area.height {
            return None;
        }

        let relative_row = (row - area.y) as usize;
        let offset = self.list_state.offset();

        match self.layout_mode {
            SidebarLayoutMode::Compact => {
                let idx = offset + relative_row;
                (idx < self.agents.len()).then_some(idx)
            }
            SidebarLayoutMode::Tiles => {
                let mut y = 0;
                for idx in offset..self.agents.len() {
                    let h = self.tile_item_height(idx);
                    if relative_row < y + h {
                        return Some(idx);
                    }
                    y += h;
                }
                None
            }
        }
    }

    /// Height in rows of a tile-mode item at the given index.
    /// Uses cached heights from the last render pass.
    fn tile_item_height(&self, idx: usize) -> usize {
        let base = self.tile_heights.get(idx).copied().unwrap_or(3);
        let mut h = base;
        if idx > 0 {
            h += 1; // top separator
        }
        if idx == self.agents.len() - 1 {
            h += 1; // bottom separator
        }
        h
    }

    pub fn jump_to_selected(&mut self) {
        if let Some(idx) = self.list_state.selected()
            && let Some(agent) = self.agents.get(idx)
        {
            let pane_id = agent.pane_id.clone();
            let _ = self.mux.switch_to_pane(&pane_id, None);
            // Signal daemon directly to bypass tmux hook round-trip latency
            super::daemon_ctrl::signal_daemon();
        }
    }

    pub fn toggle_layout_mode(&mut self) {
        self.layout_mode = match self.layout_mode {
            SidebarLayoutMode::Compact => SidebarLayoutMode::Tiles,
            SidebarLayoutMode::Tiles => SidebarLayoutMode::Compact,
        };
        // Persist to tmux so all sidebar instances pick it up immediately
        let _ = Cmd::new("tmux")
            .args(&[
                "set-option",
                "-g",
                "@workmux_sidebar_layout",
                self.layout_mode.as_str(),
            ])
            .run();
        // Persist to settings.json so it survives tmux restarts
        if let Ok(store) = crate::state::StateStore::new()
            && let Ok(mut settings) = store.load_settings()
        {
            settings.sidebar_layout = Some(self.layout_mode.as_str().to_string());
            let _ = store.save_settings(&settings);
        }
    }

    /// Toggle the sleeping state of the selected agent.
    /// Does a read-modify-write on the tmux global option so concurrent
    /// toggles from different sidebar clients don't clobber each other.
    pub fn toggle_sleeping(&mut self) {
        let Some(pane_id) = self
            .list_state
            .selected()
            .and_then(|i| self.agents.get(i))
            .map(|a| a.pane_id.clone())
        else {
            return;
        };

        // Read current set from tmux (source of truth) to avoid losing
        // toggles made by other sidebar clients since our last snapshot.
        let mut current: std::collections::HashSet<String> = Cmd::new("tmux")
            .args(&["show-option", "-gqv", "@workmux_sleeping_panes"])
            .run_and_capture_stdout()
            .ok()
            .map(|s| s.split_whitespace().map(String::from).collect())
            .unwrap_or_default();

        if !current.insert(pane_id.clone()) {
            current.remove(&pane_id);
        }

        // Update local state for immediate rendering
        self.sleeping_pane_ids = current.clone();

        // Write back to tmux
        let panes: String = current.into_iter().collect::<Vec<_>>().join(" ");
        if panes.is_empty() {
            let _ = Cmd::new("tmux")
                .args(&["set-option", "-gu", "@workmux_sleeping_panes"])
                .run();
        } else {
            let _ = Cmd::new("tmux")
                .args(&["set-option", "-g", "@workmux_sleeping_panes", &panes])
                .run();
        }

        // Signal daemon for immediate refresh (re-sort + broadcast)
        super::daemon_ctrl::signal_daemon();
    }

    pub fn window_prefix(&self) -> &str {
        &self.window_prefix
    }

    /// Resolve the (primary, secondary) label pair for an agent row.
    ///
    /// Strips the workmux prefix from session/window names so the resolver only
    /// considers user-authored values. The window name is never promoted for
    /// non-tmux backends (signaled by `window_cmd: None`).
    pub fn resolve_agent_labels(&self, agent: &AgentPane) -> (String, String) {
        let project = extract_project_name(&agent.path);
        let (worktree, _is_main) = extract_worktree_name(
            &agent.session,
            &agent.window_name,
            &self.window_prefix,
            &agent.path,
        );

        // Workmux-managed names start with the configured prefix; treat them as
        // not user-authored by clearing them before the resolver sees them.
        let session = if agent.session.starts_with(&self.window_prefix) {
            ""
        } else {
            agent.session.as_str()
        };
        let window = if agent.window_name.starts_with(&self.window_prefix) {
            ""
        } else {
            agent.window_name.as_str()
        };

        resolve_labels(
            &project,
            session,
            &worktree,
            window,
            agent.window_cmd.as_deref(),
        )
    }
}

fn parse_templates(config: &Config) -> ParsedTemplates {
    let compact_str = config
        .sidebar
        .templates
        .as_ref()
        .and_then(|t| t.compact.as_deref())
        .unwrap_or(DEFAULT_COMPACT_TEMPLATE);

    let compact = match parse_line(compact_str) {
        Ok(tokens) => tokens,
        Err(e) => {
            tracing::warn!("failed to parse compact template: {}, using default", e);
            parse_line(DEFAULT_COMPACT_TEMPLATE).expect("default template is valid")
        }
    };

    let tile_strs: Vec<&str> = config
        .sidebar
        .templates
        .as_ref()
        .and_then(|t| t.tiles.as_ref())
        .map(|v| v.iter().map(|s| s.as_str()).collect())
        .unwrap_or_else(|| DEFAULT_TILE_TEMPLATES.to_vec());

    let mut tiles = Vec::new();
    for line in tile_strs {
        match parse_line(line) {
            Ok(tokens) => tiles.push(tokens),
            Err(e) => {
                tracing::warn!("failed to parse tile template '{}': {}, skipping", line, e);
            }
        }
    }

    // If all tile templates failed, use defaults
    if tiles.is_empty() {
        tiles = DEFAULT_TILE_TEMPLATES
            .iter()
            .map(|s| parse_line(s).expect("default template is valid"))
            .collect();
    }

    ParsedTemplates { compact, tiles }
}

/// Detect this sidebar's host window using TMUX_PANE (stable, one-time).
/// Returns (session, window_id).
fn detect_host_window() -> (Option<String>, Option<String>) {
    let pane_id = std::env::var("TMUX_PANE").ok().unwrap_or_default();
    let mut args = vec!["display-message", "-p"];
    if !pane_id.is_empty() {
        args.extend_from_slice(&["-t", &pane_id]);
    }
    args.push("#{session_name}\t#{window_id}");
    let output = Cmd::new("tmux")
        .args(&args)
        .run_and_capture_stdout()
        .ok()
        .unwrap_or_default();
    let trimmed = output.trim();
    let mut parts = trimmed
        .split('\t')
        .map(|s| (!s.is_empty()).then(|| s.to_string()));
    let session = parts.next().flatten();
    let window_id = parts.next().flatten();
    (session, window_id)
}
