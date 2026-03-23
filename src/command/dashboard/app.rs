//! Application state and business logic for the dashboard TUI.

use anyhow::Result;
use ratatui::style::Style;
use ratatui::widgets::TableState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::git::{self, GitStatus};
use crate::github::PrSummary;
use crate::multiplexer::{AgentPane, AgentStatus, Multiplexer};
use crate::state::StateStore;
use crate::workflow;
use crate::workflow::types::WorktreeInfo;

use super::ui::theme::ThemePalette;

const PR_FETCH_INTERVAL: Duration = Duration::from_secs(30);

use super::agent;
use super::diff::DiffView;
use super::scope::ScopeMode;
use super::settings::{
    load_hide_stale, load_last_pane_id, load_preview_size, save_hide_stale, save_last_pane_id,
    save_preview_size,
};
use super::sort::SortMode;
use super::spinner::SPINNER_FRAMES;

/// Number of lines to capture from the agent's terminal for preview (scrollable history)
pub const PREVIEW_LINES: u16 = 200;

/// Unified event type for the dashboard event loop.
/// All background threads and the input thread send events through a single channel.
pub enum AppEvent {
    /// Terminal input event (from dedicated input thread)
    Terminal(crossterm::event::Event),
    /// Git status update for a worktree path
    GitStatus(PathBuf, GitStatus),
    /// PR status update for a repo root
    PrStatus(PathBuf, HashMap<String, PrSummary>),
    /// Full worktree list from background fetch
    WorktreeList(Vec<WorktreeInfo>),
    /// Git log preview for a worktree path
    WorktreeLog(PathBuf, String),
}

/// Which tab is active in the dashboard
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DashboardTab {
    #[default]
    Agents,
    Worktrees,
}

/// Current view mode of the dashboard
#[derive(Debug, Default, PartialEq)]
pub enum ViewMode {
    #[default]
    Dashboard,
    Diff(Box<DiffView>),
}

/// A candidate worktree for bulk sweep cleanup.
pub struct SweepCandidate {
    pub handle: String,
    pub path: PathBuf,
    pub reason: SweepReason,
    pub is_dirty: bool,
    pub selected: bool,
}

/// Why a worktree is a sweep candidate.
#[derive(Clone)]
pub enum SweepReason {
    PrMerged,
    PrClosed,
    UpstreamGone,
    MergedLocally,
}

impl SweepReason {
    pub fn label(&self) -> &'static str {
        match self {
            SweepReason::PrMerged => "PR merged",
            SweepReason::PrClosed => "PR closed",
            SweepReason::UpstreamGone => "upstream gone",
            SweepReason::MergedLocally => "merged locally",
        }
    }
}

/// State for the bulk sweep modal.
pub struct SweepState {
    pub candidates: Vec<SweepCandidate>,
    pub cursor: usize,
}

/// Plan for a pending worktree removal (shown in confirmation modal).
pub struct RemovePlan {
    pub handle: String,
    pub path: PathBuf,
    pub is_dirty: bool,
    pub is_unmerged: bool,
    pub keep_branch: bool,
    pub force_armed: bool,
}

/// App state for the TUI
pub struct App {
    /// The multiplexer backend
    pub mux: Arc<dyn Multiplexer>,
    pub agents: Vec<AgentPane>,
    /// Full agent list before name/stale filtering (populated by refresh())
    all_agents: Vec<AgentPane>,
    pub table_state: TableState,
    /// Track the selected item by pane_id to preserve selection across reorders
    selected_pane_id: Option<String>,
    /// The directory from which the dashboard was launched (used to indicate the active worktree).
    pub current_worktree: Option<PathBuf>,
    pub stale_threshold_secs: u64,
    pub config: Config,
    pub should_quit: bool,
    pub should_jump: bool,
    pub sort_mode: SortMode,
    /// Current view mode (Dashboard or Diff modal)
    pub view_mode: ViewMode,
    /// Cached preview of the currently selected agent's terminal output
    pub preview: Option<String>,
    /// Track which pane_id the preview was captured from (to detect selection changes)
    preview_pane_id: Option<String>,
    /// Input mode: keystrokes are sent directly to the selected agent's pane
    pub input_mode: bool,
    /// Manual scroll offset for the preview (None = auto-scroll to bottom)
    pub preview_scroll: Option<u16>,
    /// Number of lines in the current preview content
    pub preview_line_count: u16,
    /// Height of the preview area (updated during rendering)
    pub preview_height: u16,
    /// Git status for each worktree path
    pub git_statuses: HashMap<PathBuf, GitStatus>,
    /// Last time git status was fetched (to throttle background fetches)
    last_git_fetch: std::time::Instant,
    /// Flag to track if a git fetch is in progress (prevents thread pile-up)
    pub is_git_fetching: Arc<AtomicBool>,
    /// PR info indexed by repo root, then branch name
    pr_statuses: HashMap<PathBuf, HashMap<String, PrSummary>>,
    /// Last PR fetch time
    last_pr_fetch: std::time::Instant,
    /// Flag to prevent concurrent PR fetches
    is_pr_fetching: Arc<AtomicBool>,
    /// Unified event sender (cloned by all background threads)
    pub event_tx: mpsc::Sender<AppEvent>,
    /// Cache of repo roots for agent paths
    repo_roots: HashMap<PathBuf, PathBuf>,
    /// Frame counter for spinner animation (increments each tick)
    pub spinner_frame: u8,
    /// Whether to hide stale agents from the list
    pub hide_stale: bool,
    /// Whether to show the help overlay
    pub show_help: bool,
    /// Preview pane size as percentage (1-90). Higher = larger preview.
    pub preview_size: u8,
    /// Last jumped-to pane_id for quick toggle (cached from settings)
    last_pane_id: Option<String>,
    /// Color palette based on the configured theme
    pub palette: ThemePalette,
    /// Current color scheme
    pub scheme: crate::config::ThemeScheme,
    /// Current theme mode (dark/light)
    pub theme_mode: crate::config::ThemeMode,
    /// Path to the project config file (for persisting theme changes)
    config_path: Option<PathBuf>,
    /// Dashboard scope filter mode (All or Session)
    pub scope_mode: ScopeMode,
    /// Session name at launch time (for session scope filtering)
    launch_session: Option<String>,
    /// Whether the filter input is active (accepting keystrokes)
    pub filter_active: bool,
    /// Text filter for filtering agents by name. Empty string means no filter.
    pub filter_text: String,
    /// Pane ID awaiting kill confirmation (set when pressing x on a working agent)
    pub pending_kill_pane_id: Option<String>,
    /// Which tab is active (Agents or Worktrees)
    pub active_tab: DashboardTab,
    /// Worktree list (populated from background thread)
    pub worktrees: Vec<WorktreeInfo>,
    /// Table state for the worktree view
    pub worktree_table_state: TableState,
    /// Track selected worktree by path for stable selection
    selected_worktree_path: Option<PathBuf>,
    /// Filter text for worktree view (separate from agent filter)
    pub worktree_filter_text: String,
    /// Whether worktree filter input is active
    pub worktree_filter_active: bool,
    /// Pending worktree removal (shown in confirmation modal)
    pub pending_remove: Option<RemovePlan>,
    /// Pending bulk sweep state (shown in sweep modal)
    pub pending_sweep: Option<SweepState>,
    /// Flag to prevent concurrent worktree fetches
    is_worktree_fetching: Arc<AtomicBool>,
    /// Last time worktree list was fetched
    last_worktree_fetch: std::time::Instant,
    /// Cached git log preview for selected worktree
    pub worktree_preview: Option<String>,
    /// Path of the worktree whose preview is cached
    worktree_preview_path: Option<PathBuf>,
}

impl App {
    pub fn new(
        mux: Arc<dyn Multiplexer>,
        cli_session_filter: bool,
        event_tx: mpsc::Sender<AppEvent>,
    ) -> Result<Self> {
        let config = Config::load(None)?;

        // Get the active pane's directory to indicate the active worktree.
        // Try multiplexer first (handles popup case), fall back to current_dir.
        let current_worktree = mux
            .get_client_active_pane_path()
            .or_else(|_| std::env::current_dir())
            .ok();

        // Preview size: CLI override > tmux saved > config default
        // Clamp to 10-90 to handle manually corrupted tmux variables
        let preview_size = load_preview_size()
            .unwrap_or_else(|| config.dashboard.preview_size())
            .clamp(10, 90);

        // Determine theme mode: config override or auto-detect from terminal
        let theme_mode = config
            .theme
            .mode
            .unwrap_or_else(|| match terminal_light::luma() {
                Ok(luma) if luma > 0.6 => crate::config::ThemeMode::Light,
                _ => crate::config::ThemeMode::Dark,
            });
        let scheme = config.theme.scheme;
        let palette = ThemePalette::for_scheme(scheme, theme_mode);
        let config_path = crate::config::global_config_path();
        let sort_mode = SortMode::load();
        let scope_mode = if cli_session_filter {
            ScopeMode::Session
        } else {
            ScopeMode::load()
        };
        let launch_session = mux.current_session();
        let git_statuses = git::load_status_cache();
        let pr_statuses = crate::github::load_pr_cache();
        let hide_stale = load_hide_stale();
        let last_pane_id = load_last_pane_id();

        let mut app = Self {
            mux,
            agents: Vec::new(),
            all_agents: Vec::new(),
            table_state: TableState::default(),
            selected_pane_id: None,
            current_worktree,
            stale_threshold_secs: 60 * 60, // 60 minutes
            config,
            should_quit: false,
            should_jump: false,
            sort_mode,
            view_mode: ViewMode::default(),
            preview: None,
            preview_pane_id: None,
            input_mode: false,
            preview_scroll: None,
            preview_line_count: 0,
            preview_height: 0,
            git_statuses,
            // Set to past to trigger immediate fetch on first refresh
            last_git_fetch: std::time::Instant::now() - Duration::from_secs(60),
            is_git_fetching: Arc::new(AtomicBool::new(false)),
            pr_statuses,
            // Set to past to trigger immediate fetch on first refresh
            last_pr_fetch: std::time::Instant::now() - PR_FETCH_INTERVAL,
            is_pr_fetching: Arc::new(AtomicBool::new(false)),
            event_tx,
            repo_roots: HashMap::new(),
            spinner_frame: 0,
            hide_stale,
            show_help: false,
            preview_size,
            last_pane_id,
            palette,
            scheme,
            theme_mode,
            config_path,
            scope_mode,
            launch_session,
            filter_active: false,
            filter_text: String::new(),
            pending_kill_pane_id: None,
            active_tab: DashboardTab::Agents,
            worktrees: Vec::new(),
            worktree_table_state: TableState::default(),
            selected_worktree_path: None,
            worktree_filter_text: String::new(),
            worktree_filter_active: false,
            pending_remove: None,
            pending_sweep: None,
            is_worktree_fetching: Arc::new(AtomicBool::new(false)),
            // Set to past so first switch triggers immediate fetch
            last_worktree_fetch: std::time::Instant::now() - Duration::from_secs(60),
            worktree_preview: None,
            worktree_preview_path: None,
        };

        app.refresh();

        // Select first item if available
        if !app.agents.is_empty() {
            app.table_state.select(Some(0));
            app.selected_pane_id = app.agents.first().map(|a| a.pane_id.clone());
        }

        // Initial preview fetch
        app.update_preview();

        Ok(app)
    }

    pub fn refresh(&mut self) {
        // Load agents from StateStore with reconciliation against live pane state
        self.all_agents = StateStore::new()
            .and_then(|store| store.load_reconciled_agents(self.mux.as_ref()))
            .unwrap_or_default();

        // Apply session scope filter before sorting and background fetches
        if self.scope_mode == ScopeMode::Session
            && let Some(ref session) = self.launch_session
        {
            self.all_agents.retain(|a| a.session == *session);
        }

        // Cache repo roots for new agents (parallel execution)
        let paths_to_resolve: Vec<PathBuf> = self
            .all_agents
            .iter()
            .filter(|a| !self.repo_roots.contains_key(&a.path))
            .map(|a| a.path.clone())
            .collect();

        if !paths_to_resolve.is_empty() {
            // Resolve repo roots in parallel using threads
            let results: Vec<_> = paths_to_resolve
                .into_iter()
                .map(|path| {
                    std::thread::spawn(move || {
                        let root = git::get_repo_root_for(&path).ok();
                        (path, root)
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .filter_map(|handle| handle.join().ok())
                .collect();

            for (path, root) in results {
                if let Some(r) = root {
                    self.repo_roots.insert(path, r);
                }
            }
        }

        // Trigger background git status fetch every 5 seconds
        if self.last_git_fetch.elapsed() >= Duration::from_secs(5) {
            self.last_git_fetch = std::time::Instant::now();
            self.spawn_git_status_fetch();
        }

        // Trigger PR fetch every 30 seconds
        if self.last_pr_fetch.elapsed() >= PR_FETCH_INTERVAL {
            self.last_pr_fetch = std::time::Instant::now();
            self.spawn_pr_status_fetch();
        }

        // Trigger background worktree fetch every 5 seconds
        if self.active_tab == DashboardTab::Worktrees
            && self.last_worktree_fetch.elapsed() >= Duration::from_secs(5)
        {
            self.last_worktree_fetch = std::time::Instant::now();
            self.spawn_worktree_fetch();
        }

        // Apply name filter, stale filter, sort, and restore selection
        self.apply_filters();
    }

    /// Apply name and stale filters to the cached agent list, sort, and restore selection.
    /// This is fast (in-memory only) and safe to call on every filter keystroke.
    pub fn apply_filters(&mut self) {
        self.agents = self.all_agents.clone();

        // Apply name filter if active
        if !self.filter_text.is_empty() {
            let filter_lower = self.filter_text.to_lowercase();
            let window_prefix = self.config.window_prefix();
            self.agents.retain(|a| {
                let project = Self::extract_project_name(a).to_lowercase();
                let (worktree, _) =
                    agent::extract_worktree_name(&a.session, &a.window_name, window_prefix);
                let worktree_lower = worktree.to_lowercase();
                project.contains(&filter_lower) || worktree_lower.contains(&filter_lower)
            });
        }

        // Filter out stale agents if hide_stale is enabled
        if self.hide_stale {
            let threshold = self.stale_threshold_secs;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            self.agents.retain(|agent| {
                agent
                    .status_ts
                    .map(|ts| now.saturating_sub(ts) <= threshold)
                    .unwrap_or(true)
            });
        }

        self.sort_agents();

        // Restore selection by pane_id to follow the item across reorders
        if let Some(ref pane_id) = self.selected_pane_id {
            if let Some(new_idx) = self.agents.iter().position(|a| &a.pane_id == pane_id) {
                self.table_state.select(Some(new_idx));
            } else {
                self.selected_pane_id = None;
                if self.agents.is_empty() {
                    self.table_state.select(None);
                } else if let Some(selected) = self.table_state.selected() {
                    if selected >= self.agents.len() {
                        self.table_state.select(Some(self.agents.len() - 1));
                    }
                    if let Some(idx) = self.table_state.selected() {
                        self.selected_pane_id = self.agents.get(idx).map(|a| a.pane_id.clone());
                    }
                }
            }
        } else if let Some(selected) = self.table_state.selected() {
            if selected >= self.agents.len() {
                self.table_state.select(if self.agents.is_empty() {
                    None
                } else {
                    Some(self.agents.len() - 1)
                });
            }
            if let Some(idx) = self.table_state.selected() {
                self.selected_pane_id = self.agents.get(idx).map(|a| a.pane_id.clone());
            }
        }

        // Fallback: if nothing is selected but agents exist, select the first one.
        // This handles the case where filtering produced zero matches (clearing selection)
        // and then results reappear.
        if self.selected_pane_id.is_none()
            && self.table_state.selected().is_none()
            && !self.agents.is_empty()
        {
            self.table_state.select(Some(0));
            self.selected_pane_id = self.agents.first().map(|a| a.pane_id.clone());
        }

        self.update_preview();
    }

    /// Spawn a background thread to fetch git status for all agent worktrees
    fn spawn_git_status_fetch(&self) {
        // Skip if a fetch is already in progress (prevents thread pile-up)
        if self
            .is_git_fetching
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let tx = self.event_tx.clone();
        let is_fetching = self.is_git_fetching.clone();
        // Include both agent paths and worktree paths so the worktree view gets git status too
        let mut paths: Vec<PathBuf> = self.all_agents.iter().map(|a| a.path.clone()).collect();
        for wt in &self.worktrees {
            if !paths.contains(&wt.path) {
                paths.push(wt.path.clone());
            }
        }

        std::thread::spawn(move || {
            // Reset flag when thread completes (even on panic)
            struct ResetFlag(Arc<AtomicBool>);
            impl Drop for ResetFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = ResetFlag(is_fetching);

            for path in paths {
                let status = git::get_git_status(&path);
                let _ = tx.send(AppEvent::GitStatus(path, status));
            }
        });
    }

    /// Spawn a background thread to fetch PR status for all repos
    fn spawn_pr_status_fetch(&self) {
        // Skip if already fetching
        if self
            .is_pr_fetching
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        // Collect repo roots that have at least one non-main feature branch
        let repo_roots: std::collections::HashSet<PathBuf> = self
            .agents
            .iter()
            .filter(|agent| {
                let Some(status) = self.git_statuses.get(&agent.path) else {
                    return false;
                };
                let Some(ref branch) = status.branch else {
                    return false;
                };
                // Skip main/master - they don't need PR status
                branch != "main" && branch != "master"
            })
            .filter_map(|agent| self.repo_roots.get(&agent.path).cloned())
            .collect();

        let tx = self.event_tx.clone();
        let is_fetching = self.is_pr_fetching.clone();

        std::thread::spawn(move || {
            struct ResetFlag(Arc<AtomicBool>);
            impl Drop for ResetFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = ResetFlag(is_fetching);

            for repo_root in repo_roots {
                match crate::github::list_prs_in_repo(&repo_root) {
                    Ok(prs) => {
                        let _ = tx.send(AppEvent::PrStatus(repo_root, prs));
                    }
                    Err(e) => {
                        tracing::warn!("Failed to fetch PRs for {:?}: {}", repo_root, e);
                    }
                }
            }
        });
    }

    /// Update the preview for the currently selected agent.
    /// Only fetches if the selection has changed or preview is stale.
    pub fn update_preview(&mut self) {
        if !self.mux.supports_preview() {
            return;
        }
        let current_pane_id = self
            .table_state
            .selected()
            .and_then(|idx| self.agents.get(idx))
            .map(|agent| agent.pane_id.clone());

        // Only fetch if selection changed
        if current_pane_id != self.preview_pane_id {
            self.preview_pane_id = current_pane_id.clone();
            self.preview = current_pane_id
                .as_ref()
                .and_then(|pane_id| self.mux.capture_pane(pane_id, PREVIEW_LINES));
            // Reset scroll position when selection changes
            self.preview_scroll = None;
        }
    }

    /// Force refresh the preview (used on periodic refresh)
    pub fn refresh_preview(&mut self) {
        if !self.mux.supports_preview() {
            return;
        }
        self.preview = self
            .preview_pane_id
            .as_ref()
            .and_then(|pane_id| self.mux.capture_pane(pane_id, PREVIEW_LINES));
    }

    /// Parse pane_id to a number for proper ordering.
    /// Handles tmux format (%0, %10) and numeric formats (WezTerm, kitty).
    /// Uses u64 since kitty pane IDs can exceed u32 range.
    fn parse_pane_id(pane_id: &str) -> u64 {
        pane_id
            .strip_prefix('%')
            .unwrap_or(pane_id)
            .parse()
            .unwrap_or(u64::MAX)
    }

    /// Sort agents based on the current sort mode
    fn sort_agents(&mut self) {
        let stale_threshold = self.stale_threshold_secs;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Helper closure to get status priority (lower = higher priority)
        let get_priority = |agent: &AgentPane| -> u8 {
            let is_stale = agent
                .status_ts
                .map(|ts| now.saturating_sub(ts) > stale_threshold)
                .unwrap_or(false);

            if is_stale {
                return 3; // Stale: lowest priority
            }

            match agent.status {
                Some(AgentStatus::Waiting) => 0, // Waiting: needs input
                Some(AgentStatus::Done) => 1,    // Done: needs review
                Some(AgentStatus::Working) => 2, // Working: no action needed
                None => 3,                       // Unknown/other: lowest priority
            }
        };

        // Helper closure to get elapsed time (lower = more recent)
        let get_elapsed = |agent: &AgentPane| -> u64 {
            agent
                .status_ts
                .map(|ts| now.saturating_sub(ts))
                .unwrap_or(u64::MAX)
        };

        // Helper closure to get numeric pane_id for stable ordering
        let pane_num = |agent: &AgentPane| Self::parse_pane_id(&agent.pane_id);

        // Use sort_by_cached_key for better performance (calls key fn O(N) times vs O(N log N))
        // Include pane_id as final tiebreaker for stable ordering within groups
        match self.sort_mode {
            SortMode::Priority => {
                // Sort by priority, then by elapsed time (most recent first), then by pane_id
                self.agents
                    .sort_by_cached_key(|a| (get_priority(a), get_elapsed(a), pane_num(a)));
            }
            SortMode::Project => {
                // Sort by project name first, then by status priority within each project
                self.agents.sort_by_cached_key(|a| {
                    (Self::extract_project_name(a), get_priority(a), pane_num(a))
                });
            }
            SortMode::Recency => {
                self.agents
                    .sort_by_cached_key(|a| (get_elapsed(a), pane_num(a)));
            }
            SortMode::Natural => {
                self.agents.sort_by_cached_key(pane_num);
            }
        }
    }

    /// Cycle to the next sort mode, re-sort, and persist to tmux
    pub fn cycle_color_scheme(&mut self) {
        self.scheme = self.scheme.next();
        self.palette = ThemePalette::for_scheme(self.scheme, self.theme_mode);
        self.save_theme_scheme();
    }

    fn save_theme_scheme(&self) {
        let Some(ref path) = self.config_path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let contents = std::fs::read_to_string(path).unwrap_or_default();
        let result = update_theme_in_config(&contents, self.scheme, self.config.theme.mode);
        let _ = std::fs::write(path, result);
    }

    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.next();
        self.sort_mode.save();
        self.sort_agents();
    }

    /// Toggle between showing all agents or only the current session's agents
    pub fn toggle_scope_mode(&mut self) {
        self.scope_mode = self.scope_mode.toggle();
        self.scope_mode.save();
        self.refresh();
    }

    /// Toggle hiding stale agents
    pub fn toggle_stale_filter(&mut self) {
        self.hide_stale = !self.hide_stale;
        save_hide_stale(self.hide_stale);
        self.refresh();
    }

    /// Increase preview size by 10% (max 90%)
    pub fn increase_preview_size(&mut self) {
        self.preview_size = (self.preview_size + 10).min(90);
        save_preview_size(self.preview_size);
    }

    /// Decrease preview size by 10% (min 10%)
    pub fn decrease_preview_size(&mut self) {
        self.preview_size = self.preview_size.saturating_sub(10).max(10);
        save_preview_size(self.preview_size);
    }

    pub fn next(&mut self) {
        if self.agents.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i >= self.agents.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
        self.selected_pane_id = self.agents.get(i).map(|a| a.pane_id.clone());
        self.update_preview();
    }

    pub fn previous(&mut self) {
        if self.agents.is_empty() {
            return;
        }
        let i = match self.table_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.agents.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.table_state.select(Some(i));
        self.selected_pane_id = self.agents.get(i).map(|a| a.pane_id.clone());
        self.update_preview();
    }

    /// Switch to a pane and track the previous pane for toggle feature.
    /// This is the single source of truth for all pane switching.
    fn switch_to_pane_and_track(&mut self, target_pane_id: &str) {
        // Get the REAL current pane from the multiplexer (not UI state)
        let current_pane = self.mux.active_pane_id();

        // Attempt the switch first - only update state on success
        // Look up window_name for the target pane (needed by Zellij)
        let window_hint = self
            .agents
            .iter()
            .find(|a| a.pane_id == target_pane_id)
            .map(|a| a.window_name.as_str());
        if self
            .mux
            .switch_to_pane(target_pane_id, window_hint)
            .is_err()
        {
            return;
        }

        // Exit dashboard after jump (or keep open, depending on multiplexer)
        if self.mux.should_exit_on_jump() {
            self.should_jump = true;
        }

        // Only update last_pane_id if:
        // 1. We actually moved to a different pane
        // 2. The previous pane was an agent pane (not just any tmux pane)
        if let Some(ref current) = current_pane
            && current != target_pane_id
            && self.agents.iter().any(|a| a.pane_id == *current)
        {
            self.last_pane_id = Some(current.clone());
            save_last_pane_id(current);
        }
    }

    pub fn jump_to_selected(&mut self) {
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            let target = agent.pane_id.clone();
            self.switch_to_pane_and_track(&target);
        }
    }

    pub fn jump_to_index(&mut self, index: usize) {
        if index < self.agents.len() {
            self.table_state.select(Some(index));
            self.selected_pane_id = self.agents.get(index).map(|a| a.pane_id.clone());
            self.jump_to_selected();
        }
    }

    /// Jump to the last visited agent (toggle behavior).
    /// Reloads from settings to pick up changes from CLI command.
    pub fn jump_to_last(&mut self) {
        // Reload from settings to handle CLI/dashboard interop
        self.last_pane_id = load_last_pane_id();

        let Some(ref last_id) = self.last_pane_id else {
            return;
        };
        let last_id = last_id.clone();

        // Update table selection if the pane exists in current list
        // (handles filtered/hidden agents gracefully - still switches even if not visible)
        if let Some(idx) = self.agents.iter().position(|a| a.pane_id == last_id) {
            self.table_state.select(Some(idx));
        }

        // Switch to the pane (works even if agent is filtered out of dashboard)
        self.switch_to_pane_and_track(&last_id);
    }

    pub fn peek_selected(&mut self) {
        // Switch to pane but keep popup open
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            let _ = self
                .mux
                .switch_to_pane(&agent.pane_id, Some(&agent.window_name));
            // Don't set should_jump - popup stays open
        }
    }

    /// Kill the selected agent's pane and remove it from the list.
    /// Shows a confirmation popup for working agents.
    pub fn kill_selected(&mut self) {
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            if agent.status == Some(AgentStatus::Working) {
                // Show confirmation popup
                self.pending_kill_pane_id = Some(agent.pane_id.clone());
            } else {
                self.do_kill(&agent.pane_id.clone());
            }
        }
    }

    /// Execute the pending kill confirmation.
    pub fn confirm_kill(&mut self) {
        if let Some(pane_id) = self.pending_kill_pane_id.take() {
            self.do_kill(&pane_id);
        }
    }

    /// Kill a pane and remove it from the agent list.
    fn do_kill(&mut self, pane_id: &str) {
        let _ = self.mux.kill_pane(pane_id);

        let selected = self.table_state.selected().unwrap_or(0);

        // Remove from local lists immediately for responsive UI
        self.agents.retain(|a| a.pane_id != pane_id);
        self.all_agents.retain(|a| a.pane_id != pane_id);

        // Adjust selection
        if self.agents.is_empty() {
            self.table_state.select(None);
            self.selected_pane_id = None;
        } else {
            let new_idx = selected.min(self.agents.len() - 1);
            self.table_state.select(Some(new_idx));
            self.selected_pane_id = self.agents.get(new_idx).map(|a| a.pane_id.clone());
        }

        // Force preview refresh for new selection
        self.preview_pane_id = None;
        self.update_preview();
    }

    /// Send a key to the selected agent's pane
    pub fn send_key_to_selected(&self, key: &str) {
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            let _ = self.mux.send_key(&agent.pane_id, key);
        }
    }

    /// Scroll preview up (toward older content). Returns the amount to scroll by.
    pub fn scroll_preview_up(&mut self, visible_height: u16, total_lines: u16) {
        let max_scroll = total_lines.saturating_sub(visible_height);
        let current = self.preview_scroll.unwrap_or(max_scroll);
        let half_page = visible_height / 2;
        self.preview_scroll = Some(current.saturating_sub(half_page));
    }

    /// Scroll preview down (toward newer content).
    pub fn scroll_preview_down(&mut self, visible_height: u16, total_lines: u16) {
        let max_scroll = total_lines.saturating_sub(visible_height);
        let current = self.preview_scroll.unwrap_or(max_scroll);
        let half_page = visible_height / 2;
        let new_scroll = (current + half_page).min(max_scroll);
        // If at or past max, return to auto-scroll mode
        if new_scroll >= max_scroll {
            self.preview_scroll = None;
        } else {
            self.preview_scroll = Some(new_scroll);
        }
    }

    pub fn format_duration(&self, secs: u64) -> String {
        agent::format_duration(secs)
    }

    pub fn is_stale(&self, agent: &AgentPane) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        agent::is_stale(agent.status_ts, self.stale_threshold_secs, now)
    }

    pub fn get_elapsed(&self, agent: &AgentPane) -> Option<u64> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        agent::elapsed_secs(agent.status_ts, now)
    }

    pub fn get_status_display(&self, agent: &AgentPane) -> Vec<(String, Style)> {
        let is_stale = self.is_stale(agent);

        // Map status enum to icon and color
        let (icon, base_color, is_working) = match agent.status {
            Some(AgentStatus::Working) => {
                (self.config.status_icons.working(), self.palette.info, true)
            }
            Some(AgentStatus::Waiting) => (
                self.config.status_icons.waiting(),
                self.palette.accent,
                false,
            ),
            Some(AgentStatus::Done) => {
                (self.config.status_icons.done(), self.palette.success, false)
            }
            None => ("", self.palette.text, false),
        };

        let base_style = Style::default().fg(base_color);
        let mut spans = super::ansi::parse_tmux_styles(icon, base_style);

        if is_stale {
            // Override all styling for stale agents
            let dimmed = Style::default().fg(self.palette.dimmed);
            for span in &mut spans {
                span.1 = dimmed;
            }
            spans.push((" \u{f051b}".to_string(), dimmed));
        } else if is_working {
            // Add animated spinner when agent is working
            let spinner = SPINNER_FRAMES[self.spinner_frame as usize];
            spans.push((format!(" {}", spinner), base_style));
        }

        spans
    }

    /// Extract the worktree name from an agent.
    /// Returns (worktree_name, is_main) where is_main indicates if this is the main worktree.
    pub fn extract_worktree_name(&self, agent_pane: &AgentPane) -> (String, bool) {
        agent::extract_worktree_name(
            &agent_pane.session,
            &agent_pane.window_name,
            self.config.window_prefix(),
        )
    }

    pub fn extract_project_name(agent_pane: &AgentPane) -> String {
        agent::extract_project_name(&agent_pane.path)
    }

    /// Get PR info for an agent by looking up its branch in PR statuses
    pub fn get_pr_for_agent(&self, agent: &AgentPane) -> Option<&PrSummary> {
        let repo_root = self.repo_roots.get(&agent.path)?;
        let git_status = self.git_statuses.get(&agent.path)?;
        let branch = git_status.branch.as_ref()?;
        // Don't show PRs for main/master - you merge INTO main, not FROM it
        if branch == "main" || branch == "master" {
            return None;
        }
        self.pr_statuses.get(repo_root)?.get(branch)
    }

    /// Whether a PR fetch is currently in progress
    pub fn is_pr_fetching(&self) -> bool {
        self.is_pr_fetching.load(Ordering::Relaxed)
    }

    /// Whether any agent has a matching PR (for column visibility)
    pub fn has_any_pr(&self) -> bool {
        self.agents
            .iter()
            .any(|agent| self.get_pr_for_agent(agent).is_some())
    }

    /// Get PR statuses for caching
    pub fn pr_statuses(&self) -> &HashMap<PathBuf, HashMap<String, PrSummary>> {
        &self.pr_statuses
    }

    // ── Worktree tab methods ────────────────────────────────────────

    /// Reset the worktree fetch timer to trigger an immediate refetch
    pub fn trigger_worktree_refetch(&mut self) {
        self.last_worktree_fetch = std::time::Instant::now() - Duration::from_secs(60);
    }

    /// Apply a background event to app state.
    /// Called from the main loop when an AppEvent arrives on the unified channel.
    pub fn apply_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Terminal(_) => {} // handled separately in main loop
            AppEvent::GitStatus(path, status) => {
                self.git_statuses.insert(path, status);
            }
            AppEvent::PrStatus(repo_root, prs) => {
                self.pr_statuses.insert(repo_root, prs);
            }
            AppEvent::WorktreeList(mut worktrees) => {
                worktrees.sort_by(|a, b| {
                    let proj_a = agent::extract_project_name(&a.path);
                    let proj_b = agent::extract_project_name(&b.path);
                    proj_a.cmp(&proj_b).then_with(|| a.handle.cmp(&b.handle))
                });
                self.worktrees = worktrees;
                self.apply_worktree_filters();
            }
            AppEvent::WorktreeLog(path, log) => {
                if self.worktree_preview_path.as_ref() == Some(&path) {
                    self.worktree_preview = Some(log);
                }
            }
        }
    }

    /// Switch between Agents and Worktrees tabs
    pub fn switch_tab(&mut self) {
        self.active_tab = match self.active_tab {
            DashboardTab::Agents => DashboardTab::Worktrees,
            DashboardTab::Worktrees => DashboardTab::Agents,
        };
        if self.active_tab == DashboardTab::Worktrees {
            // Trigger immediate fetch on switch
            self.last_worktree_fetch = std::time::Instant::now();
            self.spawn_worktree_fetch();
        }
    }

    /// Spawn background thread to fetch worktree list
    fn spawn_worktree_fetch(&self) {
        if self
            .is_worktree_fetching
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let tx = self.event_tx.clone();
        let is_fetching = self.is_worktree_fetching.clone();
        let config = self.config.clone();
        let mux = self.mux.clone();

        std::thread::spawn(move || {
            struct ResetFlag(Arc<AtomicBool>);
            impl Drop for ResetFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = ResetFlag(is_fetching);

            // fetch_pr_status=false: the dashboard fetches PR status separately,
            // and workflow::list's spinner would corrupt the TUI output
            if let Ok(worktrees) = workflow::list(&config, mux.as_ref(), false, &[]) {
                let _ = tx.send(AppEvent::WorktreeList(worktrees));
            }
        });
    }

    /// Apply filter text to worktree list and restore selection
    fn apply_worktree_filters(&mut self) {
        // Apply name filter
        if !self.worktree_filter_text.is_empty() {
            let filter = self.worktree_filter_text.to_lowercase();
            self.worktrees.retain(|w| {
                let handle = w.handle.to_lowercase();
                handle.contains(&filter) || w.branch.to_lowercase().contains(&filter)
            });
        }

        // Restore selection by path
        if let Some(ref path) = self.selected_worktree_path {
            if let Some(idx) = self.worktrees.iter().position(|w| &w.path == path) {
                self.worktree_table_state.select(Some(idx));
            } else {
                self.selected_worktree_path = None;
                if self.worktrees.is_empty() {
                    self.worktree_table_state.select(None);
                } else {
                    self.worktree_table_state.select(Some(0));
                }
            }
        } else if !self.worktrees.is_empty() && self.worktree_table_state.selected().is_none() {
            self.worktree_table_state.select(Some(0));
            self.selected_worktree_path = self.worktrees.first().map(|w| w.path.clone());
        }

        self.update_worktree_preview();
    }

    pub fn worktree_next(&mut self) {
        if self.worktrees.is_empty() {
            return;
        }
        let i = self.worktree_table_state.selected().unwrap_or(0);
        let next = if i >= self.worktrees.len() - 1 {
            0
        } else {
            i + 1
        };
        self.worktree_table_state.select(Some(next));
        self.selected_worktree_path = self.worktrees.get(next).map(|w| w.path.clone());
        self.update_worktree_preview();
    }

    pub fn worktree_previous(&mut self) {
        if self.worktrees.is_empty() {
            return;
        }
        let i = self.worktree_table_state.selected().unwrap_or(0);
        let prev = if i == 0 {
            self.worktrees.len() - 1
        } else {
            i - 1
        };
        self.worktree_table_state.select(Some(prev));
        self.selected_worktree_path = self.worktrees.get(prev).map(|w| w.path.clone());
        self.update_worktree_preview();
    }

    pub fn worktree_jump_to_index(&mut self, index: usize) {
        if index < self.worktrees.len() {
            self.worktree_table_state.select(Some(index));
            self.selected_worktree_path = self.worktrees.get(index).map(|w| w.path.clone());
            self.jump_to_selected_worktree();
        }
    }

    /// Show the remove confirmation modal for the selected worktree.
    /// Always shows the modal (even for clean worktrees). Skips main worktree.
    pub fn remove_selected_worktree(&mut self) {
        let Some(selected) = self.worktree_table_state.selected() else {
            return;
        };
        let Some(worktree) = self.worktrees.get(selected) else {
            return;
        };

        // Block removal of main worktree
        if worktree.is_main {
            return;
        }

        let is_dirty = git::has_uncommitted_changes(&worktree.path).unwrap_or(false);

        self.pending_remove = Some(RemovePlan {
            handle: worktree.handle.clone(),
            path: worktree.path.clone(),
            is_dirty,
            is_unmerged: worktree.has_unmerged,
            keep_branch: false,
            force_armed: false,
        });
    }

    /// Toggle keep-branch in the pending remove plan.
    pub fn toggle_remove_keep_branch(&mut self) {
        if let Some(ref mut plan) = self.pending_remove {
            plan.keep_branch = !plan.keep_branch;
        }
    }

    /// Arm force mode for dirty worktree removal.
    pub fn arm_remove_force(&mut self) {
        if let Some(ref mut plan) = self.pending_remove
            && plan.is_dirty
        {
            plan.force_armed = true;
        }
    }

    /// Execute the pending remove confirmation.
    pub fn confirm_remove(&mut self) {
        let Some(plan) = self.pending_remove.take() else {
            return;
        };

        // Dirty worktrees require force to be armed
        if plan.is_dirty && !plan.force_armed {
            self.pending_remove = Some(plan);
            return;
        }

        self.do_remove_worktree(&plan.path, plan.keep_branch);
    }

    fn do_remove_worktree(&mut self, path: &Path, keep_branch: bool) {
        let handle = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let Ok(ctx) = workflow::WorkflowContext::new(self.config.clone(), self.mux.clone(), None)
        else {
            return;
        };

        // force=true because user confirmed via modal
        if workflow::remove(&handle, true, keep_branch, &ctx).is_ok() {
            self.worktrees.retain(|w| w.path != *path);

            if self.worktrees.is_empty() {
                self.worktree_table_state.select(None);
                self.selected_worktree_path = None;
            } else {
                let idx = self.worktree_table_state.selected().unwrap_or(0);
                let new_idx = idx.min(self.worktrees.len() - 1);
                self.worktree_table_state.select(Some(new_idx));
                self.selected_worktree_path = self.worktrees.get(new_idx).map(|w| w.path.clone());
            }
        }
    }

    /// Build the sweep candidate list and open the sweep modal.
    pub fn start_sweep(&mut self) {
        let gone = git::get_gone_branches().unwrap_or_default();

        let mut candidates: Vec<SweepCandidate> = Vec::new();

        for wt in &self.worktrees {
            if wt.is_main {
                continue;
            }

            let status = self.git_statuses.get(&wt.path);
            let is_dirty = status.is_some_and(|s| s.is_dirty);
            let has_upstream = status.is_some_and(|s| s.has_upstream);

            // Determine reason: PR merged > PR closed > upstream gone > merged locally
            let reason = if let Some(ref pr) = wt.pr_info {
                match pr.state.as_str() {
                    "MERGED" => Some(SweepReason::PrMerged),
                    "CLOSED" => Some(SweepReason::PrClosed),
                    _ => {
                        if gone.contains(&wt.branch) {
                            Some(SweepReason::UpstreamGone)
                        } else {
                            None
                        }
                    }
                }
            } else if gone.contains(&wt.branch) {
                Some(SweepReason::UpstreamGone)
            } else if !has_upstream && !wt.has_unmerged {
                Some(SweepReason::MergedLocally)
            } else {
                None
            };

            let Some(reason) = reason else { continue };

            candidates.push(SweepCandidate {
                handle: wt.handle.clone(),
                path: wt.path.clone(),
                reason,
                is_dirty,
                selected: !is_dirty, // Pre-select non-dirty candidates
            });
        }

        self.pending_sweep = Some(SweepState {
            candidates,
            cursor: 0,
        });
    }

    /// Toggle selection of the current sweep candidate.
    pub fn sweep_toggle(&mut self) {
        if let Some(ref mut sweep) = self.pending_sweep
            && let Some(candidate) = sweep.candidates.get_mut(sweep.cursor)
            && !candidate.is_dirty
        {
            candidate.selected = !candidate.selected;
        }
    }

    /// Move cursor up in sweep modal.
    pub fn sweep_up(&mut self) {
        if let Some(ref mut sweep) = self.pending_sweep {
            sweep.cursor = sweep.cursor.saturating_sub(1);
        }
    }

    /// Move cursor down in sweep modal.
    pub fn sweep_down(&mut self) {
        if let Some(ref mut sweep) = self.pending_sweep
            && sweep.cursor + 1 < sweep.candidates.len()
        {
            sweep.cursor += 1;
        }
    }

    /// Execute sweep: remove all selected candidates.
    pub fn confirm_sweep(&mut self) {
        let Some(sweep) = self.pending_sweep.take() else {
            return;
        };

        let paths_to_remove: Vec<PathBuf> = sweep
            .candidates
            .iter()
            .filter(|c| c.selected)
            .map(|c| c.path.clone())
            .collect();

        for path in &paths_to_remove {
            self.do_remove_worktree(path, false);
        }
    }

    /// Jump to the selected worktree's agent or mux window
    pub fn jump_to_selected_worktree(&mut self) {
        let Some(selected) = self.worktree_table_state.selected() else {
            return;
        };
        let Some(worktree) = self.worktrees.get(selected) else {
            return;
        };

        // Try agent first
        if let Some(agent) = self.all_agents.iter().find(|a| a.path == worktree.path) {
            let target = agent.pane_id.clone();
            self.switch_to_pane_and_track(&target);
            return;
        }

        // Fall back to mux window/session via MuxHandle
        let handle = &worktree.handle;
        let prefix = self.config.window_prefix();
        let mode = worktree.mode;
        let mux_handle =
            crate::multiplexer::handle::MuxHandle::new(self.mux.as_ref(), mode, prefix, handle);
        if mux_handle.exists().unwrap_or(false) {
            let _ = mux_handle.select();
            self.should_jump = true;
        }
    }

    /// Update the preview for the selected worktree (git log)
    fn update_worktree_preview(&mut self) {
        let current_path = self
            .worktree_table_state
            .selected()
            .and_then(|idx| self.worktrees.get(idx))
            .map(|w| w.path.clone());

        if current_path != self.worktree_preview_path {
            self.worktree_preview_path = current_path.clone();
            self.worktree_preview = None;

            if let Some(path) = current_path {
                let tx = self.event_tx.clone();
                std::thread::spawn(move || {
                    let output = std::process::Command::new("git")
                        .args(["log", "--oneline", "-n", "20"])
                        .current_dir(&path)
                        .output();
                    if let Ok(out) = output {
                        let log = String::from_utf8_lossy(&out.stdout).to_string();
                        let _ = tx.send(AppEvent::WorktreeLog(path, log));
                    }
                });
            }
        }
    }
}

/// Update the `theme:` entry in a YAML config string.
/// Preserves explicit mode override when present.
/// Prefers uncommented `theme:` over `# theme:`.
fn update_theme_in_config(
    contents: &str,
    scheme: crate::config::ThemeScheme,
    explicit_mode: Option<crate::config::ThemeMode>,
) -> String {
    use crate::config::{ThemeMode, ThemeScheme};

    let slug = scheme.slug();

    // Build replacement lines
    let new_lines_for_theme = if scheme == ThemeScheme::Default && explicit_mode.is_none() {
        vec!["# theme: default".to_string()]
    } else if let Some(mode) = explicit_mode {
        let mode_str = match mode {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        };
        vec![
            "theme:".to_string(),
            format!("  scheme: {}", slug),
            format!("  mode: {}", mode_str),
        ]
    } else {
        vec![format!("theme: {}", slug)]
    };

    let lines: Vec<&str> = contents.lines().collect();

    // First pass: find the best target line (prefer uncommented over commented)
    let mut uncommented_idx = None;
    let mut commented_idx = None;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("theme:") && uncommented_idx.is_none() {
            uncommented_idx = Some(i);
        } else if trimmed.starts_with("# theme:") && commented_idx.is_none() {
            commented_idx = Some(i);
        }
    }
    let target_idx = uncommented_idx.or(commented_idx);

    // Second pass: build output
    let mut result_lines: Vec<String> = Vec::new();
    let mut replaced = false;
    let mut iter = lines.iter().enumerate().peekable();

    while let Some((i, line)) = iter.next() {
        if !replaced && Some(i) == target_idx {
            replaced = true;
            result_lines.extend(new_lines_for_theme.clone());

            // Skip structured block sub-keys (including blank lines within)
            let trimmed = line.trim_start();
            let is_block = trimmed
                .strip_prefix("theme:")
                .is_some_and(|rest| rest.trim().is_empty());

            if is_block {
                let block_indent = line.len() - trimmed.len();
                while let Some(&(_, next_line)) = iter.peek() {
                    let next_trimmed = next_line.trim_start();
                    // Continue through blank lines and deeper-indented lines
                    if next_trimmed.is_empty() {
                        iter.next();
                        continue;
                    }
                    if (next_line.len() - next_trimmed.len()) > block_indent {
                        iter.next();
                    } else {
                        break;
                    }
                }
            }
        } else {
            result_lines.push(line.to_string());
        }
    }

    if !replaced && scheme != ThemeScheme::Default {
        result_lines.extend(new_lines_for_theme);
    }

    let mut result = result_lines.join("\n");
    if contents.ends_with('\n') && !result.ends_with('\n') {
        result.push('\n');
    }
    if !contents.ends_with('\n') && result.ends_with('\n') {
        result.pop();
    }
    result
}

#[cfg(test)]
mod theme_persistence_tests {
    use super::update_theme_in_config;
    use crate::config::{ThemeMode, ThemeScheme};

    #[test]
    fn simple_theme_line() {
        let input = "agent: claude\ntheme: default\nmode: window\n";
        let result = update_theme_in_config(input, ThemeScheme::Emberforge, None);
        assert_eq!(result, "agent: claude\ntheme: emberforge\nmode: window\n");
    }

    #[test]
    fn no_theme_line_appends() {
        let input = "agent: claude\n";
        let result = update_theme_in_config(input, ThemeScheme::Lasergrid, None);
        assert_eq!(result, "agent: claude\ntheme: lasergrid\n");
    }

    #[test]
    fn no_theme_line_default_does_nothing() {
        let input = "agent: claude\n";
        let result = update_theme_in_config(input, ThemeScheme::Default, None);
        assert_eq!(result, "agent: claude\n");
    }

    #[test]
    fn default_scheme_comments_out() {
        let input = "theme: emberforge\n";
        let result = update_theme_in_config(input, ThemeScheme::Default, None);
        assert_eq!(result, "# theme: default\n");
    }

    #[test]
    fn structured_block_replaced() {
        let input = "agent: claude\ntheme:\n  scheme: emberforge\n  mode: dark\nmode: window\n";
        let result = update_theme_in_config(input, ThemeScheme::SlateGarden, None);
        assert_eq!(result, "agent: claude\ntheme: slate-garden\nmode: window\n");
    }

    #[test]
    fn structured_block_with_blank_lines() {
        let input = "agent: claude\ntheme:\n  scheme: emberforge\n\n  mode: dark\nmode: window\n";
        let result = update_theme_in_config(input, ThemeScheme::Mossfire, None);
        assert_eq!(result, "agent: claude\ntheme: mossfire\nmode: window\n");
    }

    #[test]
    fn preserves_explicit_mode() {
        let input = "theme: emberforge\n";
        let result =
            update_theme_in_config(input, ThemeScheme::GlacierSignal, Some(ThemeMode::Light));
        assert_eq!(result, "theme:\n  scheme: glacier-signal\n  mode: light\n");
    }

    #[test]
    fn preserves_explicit_dark_mode() {
        let input = "theme: default\n";
        let result = update_theme_in_config(input, ThemeScheme::ObsidianPop, Some(ThemeMode::Dark));
        assert_eq!(result, "theme:\n  scheme: obsidian-pop\n  mode: dark\n");
    }

    #[test]
    fn default_with_explicit_mode() {
        let input = "theme: emberforge\n";
        let result = update_theme_in_config(input, ThemeScheme::Default, Some(ThemeMode::Light));
        assert_eq!(result, "theme:\n  scheme: default\n  mode: light\n");
    }

    #[test]
    fn prefers_uncommented_over_commented() {
        let input = "# theme: default\nagent: claude\ntheme: emberforge\n";
        let result = update_theme_in_config(input, ThemeScheme::Lasergrid, None);
        assert_eq!(
            result,
            "# theme: default\nagent: claude\ntheme: lasergrid\n"
        );
    }

    #[test]
    fn falls_back_to_commented_if_no_active() {
        let input = "# theme: default\nagent: claude\n";
        let result = update_theme_in_config(input, ThemeScheme::NightSorbet, None);
        assert_eq!(result, "theme: night-sorbet\nagent: claude\n");
    }

    #[test]
    fn empty_file() {
        let result = update_theme_in_config("", ThemeScheme::Emberforge, None);
        assert_eq!(result, "theme: emberforge");
    }

    #[test]
    fn empty_file_default() {
        let result = update_theme_in_config("", ThemeScheme::Default, None);
        assert_eq!(result, "");
    }

    #[test]
    fn preserves_surrounding_content() {
        let input = "# my config\nagent: claude\ntheme: mossfire\nnerdfont: true\n# end\n";
        let result = update_theme_in_config(input, ThemeScheme::TealDrift, None);
        assert_eq!(
            result,
            "# my config\nagent: claude\ntheme: teal-drift\nnerdfont: true\n# end\n"
        );
    }

    #[test]
    fn structured_to_structured_preserves_mode() {
        let input = "theme:\n  scheme: emberforge\n  mode: light\n";
        let result =
            update_theme_in_config(input, ThemeScheme::FestivalCircuit, Some(ThemeMode::Light));
        assert_eq!(
            result,
            "theme:\n  scheme: festival-circuit\n  mode: light\n"
        );
    }

    #[test]
    fn no_trailing_newline_preserved() {
        let input = "theme: default";
        let result = update_theme_in_config(input, ThemeScheme::Emberforge, None);
        assert_eq!(result, "theme: emberforge");
    }
}
