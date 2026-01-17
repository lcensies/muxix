//! Application state and business logic for the dashboard TUI.

use anyhow::Result;
use ratatui::style::Color;
use ratatui::widgets::TableState;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::git::{self, GitStatus};
use crate::tmux::{self, AgentPane};

use super::agent;
use super::ansi::parse_ansi_to_lines;
use super::diff::{
    DiffView, extract_file_list, get_diff_content, get_file_list_numstat, map_file_offsets,
    parse_hunk_header,
};
use super::settings::{
    load_hide_stale_from_tmux, load_preview_size_from_tmux, save_hide_stale_to_tmux,
    save_preview_size_to_tmux,
};
use super::sort::SortMode;
use super::spinner::SPINNER_FRAMES;

/// Number of lines to capture from the agent's terminal for preview (scrollable history)
pub const PREVIEW_LINES: u16 = 200;

/// Current view mode of the dashboard
#[derive(Debug, Default, PartialEq)]
pub enum ViewMode {
    #[default]
    Dashboard,
    Diff(Box<DiffView>),
}

/// App state for the TUI
pub struct App {
    pub agents: Vec<AgentPane>,
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
    /// Channel receiver for git status updates from background thread
    git_rx: mpsc::Receiver<(PathBuf, GitStatus)>,
    /// Channel sender for git status updates (cloned for background threads)
    git_tx: mpsc::Sender<(PathBuf, GitStatus)>,
    /// Last time git status was fetched (to throttle background fetches)
    last_git_fetch: std::time::Instant,
    /// Flag to track if a git fetch is in progress (prevents thread pile-up)
    pub is_git_fetching: Arc<AtomicBool>,
    /// Frame counter for spinner animation (increments each tick)
    pub spinner_frame: u8,
    /// Whether to hide stale agents from the list
    pub hide_stale: bool,
    /// Whether to show the help overlay
    pub show_help: bool,
    /// Preview pane size as percentage (1-90). Higher = larger preview.
    pub preview_size: u8,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = Config::load(None)?;
        let (git_tx, git_rx) = mpsc::channel();
        // Get the active pane's directory to indicate the active worktree.
        // Try tmux first (handles popup case), fall back to current_dir.
        let current_worktree = crate::tmux::get_client_active_pane_path()
            .or_else(|_| std::env::current_dir())
            .ok();
        // Preview size: CLI override > tmux saved > config default
        // Clamp to 10-90 to handle manually corrupted tmux variables
        let preview_size = load_preview_size_from_tmux()
            .unwrap_or_else(|| config.dashboard.preview_size())
            .clamp(10, 90);

        let mut app = Self {
            agents: Vec::new(),
            table_state: TableState::default(),
            selected_pane_id: None,
            current_worktree,
            stale_threshold_secs: 60 * 60, // 60 minutes
            config,
            should_quit: false,
            should_jump: false,
            sort_mode: SortMode::load_from_tmux(),
            view_mode: ViewMode::default(),
            preview: None,
            preview_pane_id: None,
            input_mode: false,
            preview_scroll: None,
            preview_line_count: 0,
            preview_height: 0,
            git_statuses: git::load_status_cache(),
            git_rx,
            git_tx,
            // Set to past to trigger immediate fetch on first refresh
            last_git_fetch: std::time::Instant::now() - Duration::from_secs(60),
            is_git_fetching: Arc::new(AtomicBool::new(false)),
            spinner_frame: 0,
            hide_stale: load_hide_stale_from_tmux(),
            show_help: false,
            preview_size,
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
        self.agents = tmux::get_all_agent_panes().unwrap_or_default();
        self.sort_agents();

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
                    .unwrap_or(true) // Keep agents without timestamp
            });
        }

        // Consume any pending git status updates from background thread
        while let Ok((path, status)) = self.git_rx.try_recv() {
            self.git_statuses.insert(path, status);
        }

        // Trigger background git status fetch every 5 seconds
        if self.last_git_fetch.elapsed() >= Duration::from_secs(5) {
            self.last_git_fetch = std::time::Instant::now();
            self.spawn_git_status_fetch();
        }

        // Restore selection by pane_id to follow the item across reorders
        if let Some(ref pane_id) = self.selected_pane_id {
            // Find the new index of the previously selected item
            if let Some(new_idx) = self.agents.iter().position(|a| &a.pane_id == pane_id) {
                self.table_state.select(Some(new_idx));
            } else {
                // Item was removed (filtered out or closed), keep selection in bounds
                self.selected_pane_id = None;
                if self.agents.is_empty() {
                    self.table_state.select(None);
                } else if let Some(selected) = self.table_state.selected() {
                    if selected >= self.agents.len() {
                        self.table_state.select(Some(self.agents.len() - 1));
                    }
                    // Update selected_pane_id to the new selection
                    if let Some(idx) = self.table_state.selected() {
                        self.selected_pane_id = self.agents.get(idx).map(|a| a.pane_id.clone());
                    }
                }
            }
        } else if let Some(selected) = self.table_state.selected() {
            // No tracked pane_id but we have a selection - adjust if out of bounds
            if selected >= self.agents.len() {
                self.table_state.select(if self.agents.is_empty() {
                    None
                } else {
                    Some(self.agents.len() - 1)
                });
            }
            // Sync selected_pane_id to ensure we start tracking the current selection
            if let Some(idx) = self.table_state.selected() {
                self.selected_pane_id = self.agents.get(idx).map(|a| a.pane_id.clone());
            }
        }

        // Update preview for current selection
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

        let tx = self.git_tx.clone();
        let is_fetching = self.is_git_fetching.clone();
        let agent_paths: Vec<PathBuf> = self.agents.iter().map(|a| a.path.clone()).collect();

        std::thread::spawn(move || {
            // Reset flag when thread completes (even on panic)
            struct ResetFlag(Arc<AtomicBool>);
            impl Drop for ResetFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = ResetFlag(is_fetching);

            for path in agent_paths {
                let status = git::get_git_status(&path);
                // Ignore send errors (receiver dropped means app is shutting down)
                let _ = tx.send((path, status));
            }
        });
    }

    /// Update the preview for the currently selected agent.
    /// Only fetches if the selection has changed or preview is stale.
    pub fn update_preview(&mut self) {
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
                .and_then(|pane_id| tmux::capture_pane(pane_id, PREVIEW_LINES));
            // Reset scroll position when selection changes
            self.preview_scroll = None;
        }
    }

    /// Force refresh the preview (used on periodic refresh)
    pub fn refresh_preview(&mut self) {
        self.preview = self
            .preview_pane_id
            .as_ref()
            .and_then(|pane_id| tmux::capture_pane(pane_id, PREVIEW_LINES));
    }

    /// Parse pane_id (e.g., "%0", "%10") to a number for proper ordering
    fn parse_pane_id(pane_id: &str) -> u32 {
        pane_id
            .strip_prefix('%')
            .and_then(|s| s.parse().ok())
            .unwrap_or(u32::MAX)
    }

    /// Sort agents based on the current sort mode
    fn sort_agents(&mut self) {
        // Extract config values needed for sorting to avoid borrowing issues
        let waiting = self.config.status_icons.waiting().to_string();
        let working = self.config.status_icons.working().to_string();
        let done = self.config.status_icons.done().to_string();
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

            match agent.status.as_deref().unwrap_or("") {
                s if s == waiting => 0, // Waiting: needs input
                s if s == done => 1,    // Done: needs review
                s if s == working => 2, // Working: no action needed
                _ => 3,                 // Unknown/other: lowest priority
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
    pub fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.next();
        self.sort_mode.save_to_tmux();
        self.sort_agents();
    }

    /// Toggle hiding stale agents
    pub fn toggle_stale_filter(&mut self) {
        self.hide_stale = !self.hide_stale;
        save_hide_stale_to_tmux(self.hide_stale);
        self.refresh();
    }

    /// Increase preview size by 10% (max 90%)
    pub fn increase_preview_size(&mut self) {
        self.preview_size = (self.preview_size + 10).min(90);
        save_preview_size_to_tmux(self.preview_size);
    }

    /// Decrease preview size by 10% (min 10%)
    pub fn decrease_preview_size(&mut self) {
        self.preview_size = self.preview_size.saturating_sub(10).max(10);
        save_preview_size_to_tmux(self.preview_size);
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

    pub fn jump_to_selected(&mut self) {
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            self.should_jump = true;
            // Jump to the specific pane
            let _ = tmux::switch_to_pane(&agent.pane_id);
        }
    }

    pub fn jump_to_index(&mut self, index: usize) {
        if index < self.agents.len() {
            self.table_state.select(Some(index));
            self.selected_pane_id = self.agents.get(index).map(|a| a.pane_id.clone());
            self.jump_to_selected();
        }
    }

    pub fn peek_selected(&mut self) {
        // Switch to pane but keep popup open
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            let _ = tmux::switch_to_pane(&agent.pane_id);
            // Don't set should_jump - popup stays open
        }
    }

    /// Send a key to the selected agent's pane
    pub fn send_key_to_selected(&self, key: &str) {
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            let _ = tmux::send_key(&agent.pane_id, key);
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

    pub fn get_status_display(&self, agent: &AgentPane) -> (String, Color) {
        let status = agent.status.as_deref().unwrap_or("");
        let is_stale = self.is_stale(agent);

        // Match against configured icons
        let working = self.config.status_icons.working();
        let waiting = self.config.status_icons.waiting();
        let done = self.config.status_icons.done();

        // Get the base status text and color
        let (status_text, base_color, is_working) = if status == working {
            (status.to_string(), Color::Cyan, true)
        } else if status == waiting {
            (status.to_string(), Color::Magenta, false)
        } else if status == done {
            (status.to_string(), Color::Green, false)
        } else {
            (status.to_string(), Color::White, false)
        };

        // If stale, dim the color and add timer-off indicator
        if is_stale {
            let display_text = format!("{} \u{f051b}", status_text);
            (display_text, Color::DarkGray)
        } else if is_working {
            // Add animated spinner when agent is working
            let spinner = SPINNER_FRAMES[self.spinner_frame as usize];
            let display_text = format!("{} {}", status_text, spinner);
            (display_text, base_color)
        } else {
            (status_text, base_color)
        }
    }

    /// Extract the worktree name from an agent.
    /// Returns (worktree_name, is_main) where is_main indicates if this is the main worktree.
    pub fn extract_worktree_name(&self, agent_pane: &AgentPane) -> (String, bool) {
        agent::extract_worktree_name(&agent_pane.window_name, self.config.window_prefix())
    }

    pub fn extract_project_name(agent_pane: &AgentPane) -> String {
        agent::extract_project_name(&agent_pane.path)
    }

    /// Stage a single hunk using git apply --cached
    pub fn stage_hunk(&mut self) -> Result<(), String> {
        let ViewMode::Diff(ref diff) = self.view_mode else {
            return Err("Not in diff view".to_string());
        };

        if !diff.patch_mode || diff.hunks.is_empty() {
            return Err("Not in patch mode or no hunks".to_string());
        }

        let hunk = &diff.hunks[diff.current_hunk];
        // Hunks are clean (no ANSI codes) since we use --no-color for diff
        let patch_content = format!("{}\n{}\n", hunk.file_header, hunk.hunk_body);

        let mut child = std::process::Command::new("git")
            .arg("-C")
            .arg(&diff.worktree_path)
            .args(["apply", "--cached", "--recount", "--3way", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn git: {}", e))?;

        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin
                .write_all(patch_content.as_bytes())
                .map_err(|e| format!("Failed to write to stdin: {}", e))?;
        }

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to wait on git: {}", e))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(format!("git apply failed: {}", err));
        }

        Ok(())
    }

    /// Move to next hunk in patch mode, returns true if there are more hunks
    pub fn next_hunk(&mut self) -> bool {
        if let ViewMode::Diff(ref mut diff) = self.view_mode
            && diff.patch_mode
            && diff.current_hunk + 1 < diff.hunks.len()
        {
            diff.current_hunk += 1;
            diff.scroll = 0;
            return true;
        }
        false
    }

    /// Move to previous hunk in patch mode
    pub fn prev_hunk(&mut self) {
        if let ViewMode::Diff(ref mut diff) = self.view_mode
            && diff.patch_mode
            && diff.current_hunk > 0
        {
            diff.current_hunk -= 1;
            diff.scroll = 0;
        }
    }

    /// Enter patch mode for the current diff view
    pub fn enter_patch_mode(&mut self) {
        // Check if we are in WIP diff view (patch mode not supported for branch diffs)
        let is_wip_diff = if let ViewMode::Diff(ref diff) = self.view_mode {
            !diff.is_branch_diff
        } else {
            false
        };

        if !is_wip_diff {
            return;
        }

        // Reload the diff to show only unstaged changes.
        // This ensures we only patch hunks that aren't already staged.
        // The WIP view uses `git diff HEAD` which shows all uncommitted changes,
        // but patch mode should only show unstaged changes (like `git add -p`).
        self.reload_unstaged_diff();

        // Enable patch mode if there are unstaged hunks
        if let ViewMode::Diff(ref mut diff) = self.view_mode {
            if diff.hunks.is_empty() {
                return;
            }
            diff.patch_mode = true;
            diff.current_hunk = 0;
            diff.scroll = 0;
            // Initialize progress tracking
            diff.hunks_total = diff.hunks.len();
            diff.hunks_processed = 0;
            diff.staged_hunks.clear();
        }
    }

    /// Exit patch mode back to normal diff view
    pub fn exit_patch_mode(&mut self) {
        if let ViewMode::Diff(ref mut diff) = self.view_mode {
            diff.patch_mode = false;
            diff.scroll = 0;
        }
    }

    /// Stage current hunk and advance to next, refreshing if needed
    pub fn stage_and_next(&mut self) {
        if let Err(e) = self.stage_hunk() {
            // TODO: Show error to user
            eprintln!("Failed to stage hunk: {}", e);
            return;
        }

        // Remove the staged hunk from the in-memory list and advance
        // Don't reload from git immediately - this preserves split hunks
        let should_reload = if let ViewMode::Diff(ref mut diff) = self.view_mode {
            if !diff.hunks.is_empty() {
                // Save the staged hunk for undo functionality
                let staged_hunk = diff.hunks.remove(diff.current_hunk);
                diff.staged_hunks.push(staged_hunk);
                diff.hunks_processed += 1;
                // Adjust index if we were at the end
                if diff.current_hunk >= diff.hunks.len() && !diff.hunks.is_empty() {
                    diff.current_hunk = diff.hunks.len() - 1;
                }
                diff.scroll = 0;
            }
            diff.hunks.is_empty()
        } else {
            false
        };

        if should_reload {
            // No more hunks in memory - reload to check for any remaining unstaged changes
            self.reload_unstaged_diff();

            // Re-enter patch mode if git found more hunks
            if let ViewMode::Diff(ref mut diff) = self.view_mode {
                if !diff.hunks.is_empty() {
                    diff.patch_mode = true;
                    diff.current_hunk = 0;
                } else {
                    diff.patch_mode = false;
                }
            }
        }
    }

    /// Reload diff showing only unstaged changes (for patch mode)
    fn reload_unstaged_diff(&mut self) {
        let (path, pane_id, worktree_name) = if let ViewMode::Diff(ref diff) = self.view_mode {
            (
                diff.worktree_path.clone(),
                diff.pane_id.clone(),
                diff.title
                    .strip_prefix("WIP: ")
                    .unwrap_or(&diff.title)
                    .to_string(),
            )
        } else {
            return;
        };

        // Use empty diff_arg for unstaged changes only (git diff without args)
        // Include untracked files, parse hunks for patch mode
        match get_diff_content(&path, "", true, true) {
            Ok((content, lines_added, lines_removed, hunks)) => {
                let (content, line_count) = if content.trim().is_empty() {
                    ("No uncommitted changes".to_string(), 1)
                } else {
                    let count = content.lines().count();
                    (content, count)
                };
                let parsed_lines = parse_ansi_to_lines(&content);
                let mut file_list = extract_file_list(&hunks);
                map_file_offsets(&mut file_list, &parsed_lines);

                self.view_mode = ViewMode::Diff(Box::new(DiffView {
                    content,
                    parsed_lines,
                    scroll: 0,
                    line_count,
                    viewport_height: 0,
                    title: format!("WIP: {}", worktree_name),
                    worktree_path: path,
                    pane_id,
                    is_branch_diff: false,
                    lines_added,
                    lines_removed,
                    patch_mode: false,
                    hunks,
                    current_hunk: 0,
                    hunks_total: 0,
                    hunks_processed: 0,
                    staged_hunks: Vec::new(),
                    comment_input: None,
                    file_list,
                }));
            }
            Err(e) => {
                let parsed_lines = parse_ansi_to_lines(&e);
                self.view_mode = ViewMode::Diff(Box::new(DiffView {
                    content: e,
                    parsed_lines,
                    scroll: 0,
                    line_count: 1,
                    viewport_height: 0,
                    title: "Error".to_string(),
                    worktree_path: path,
                    pane_id,
                    is_branch_diff: false,
                    lines_added: 0,
                    lines_removed: 0,
                    patch_mode: false,
                    hunks: Vec::new(),
                    current_hunk: 0,
                    hunks_total: 0,
                    hunks_processed: 0,
                    staged_hunks: Vec::new(),
                    comment_input: None,
                    file_list: Vec::new(),
                }));
            }
        }
    }

    /// Skip current hunk and move to next
    pub fn skip_hunk(&mut self) {
        // Increment processed count
        if let ViewMode::Diff(ref mut diff) = self.view_mode {
            diff.hunks_processed += 1;
        }
        if !self.next_hunk() {
            // No more hunks, exit patch mode
            self.exit_patch_mode();
        }
    }

    /// Undo the last staged hunk (unstage it and restore to the list)
    pub fn undo_staged_hunk(&mut self) {
        let ViewMode::Diff(ref mut diff) = self.view_mode else {
            return;
        };

        if !diff.patch_mode || diff.staged_hunks.is_empty() {
            return;
        }

        // Pop the last staged hunk
        let hunk = diff.staged_hunks.pop().unwrap();

        // Unstage it using git apply --cached --reverse
        let patch_content = format!("{}\n{}\n", hunk.file_header, hunk.hunk_body);

        let result = std::process::Command::new("git")
            .arg("-C")
            .arg(&diff.worktree_path)
            .args(["apply", "--cached", "--reverse", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(mut stdin) = child.stdin.take() {
                    use std::io::Write;
                    let _ = stdin.write_all(patch_content.as_bytes());
                }
                child.wait_with_output()
            });

        if let Ok(output) = result
            && output.status.success()
        {
            // Insert the hunk back at the current position
            diff.hunks.insert(diff.current_hunk, hunk);
            diff.hunks_processed = diff.hunks_processed.saturating_sub(1);
            diff.scroll = 0;
        }
    }

    /// Send a comment about the current hunk to the agent
    pub fn send_hunk_comment(&mut self) {
        let ViewMode::Diff(ref mut diff) = self.view_mode else {
            return;
        };

        if !diff.patch_mode || diff.hunks.is_empty() {
            return;
        }

        let comment = match diff.comment_input.take() {
            Some(c) if !c.trim().is_empty() => c,
            _ => return,
        };

        let hunk = &diff.hunks[diff.current_hunk];

        // Extract line number from hunk header (e.g., "@@ -10,5 +12,7 @@" -> 12)
        let line_num = parse_hunk_header(&hunk.hunk_body)
            .map(|(_, new_start)| new_start)
            .unwrap_or(1);

        // Determine safe code fence (use more backticks if content contains ```)
        let mut fence = "```".to_string();
        while hunk.hunk_body.contains(&fence) {
            fence.push('`');
        }

        // Format the message with file path, line number, hunk content, and comment
        let message = format!(
            "{}:{}\n\n{}diff\n{}\n{}\n\n{}",
            hunk.filename, line_num, fence, hunk.hunk_body, fence, comment
        );

        // Use paste_multiline to properly handle newlines in the message
        let _ = tmux::paste_multiline(&diff.pane_id, &message);
        // Send an additional Enter to submit the comment to the agent
        let _ = tmux::send_key(&diff.pane_id, "Enter");
    }

    /// Split the current hunk into smaller hunks if possible
    /// Returns true if the split was successful
    pub fn split_current_hunk(&mut self) -> bool {
        if let ViewMode::Diff(ref mut diff) = self.view_mode {
            if !diff.patch_mode || diff.hunks.is_empty() {
                return false;
            }

            let current_idx = diff.current_hunk;
            let current = &diff.hunks[current_idx];

            if let Some(sub_hunks) = current.split() {
                let num_new_hunks = sub_hunks.len();
                // Remove the original hunk and insert the split hunks
                diff.hunks.remove(current_idx);
                for (i, h) in sub_hunks.into_iter().enumerate() {
                    diff.hunks.insert(current_idx + i, h);
                }
                // Adjust total to account for the split (one hunk became num_new_hunks)
                diff.hunks_total += num_new_hunks - 1;
                // Stay at the first split hunk, reset scroll
                diff.scroll = 0;
                return num_new_hunks > 1;
            }
        }
        false
    }

    /// Load diff for the selected worktree
    /// - `branch_diff`: if true, diff against main branch; if false, diff HEAD (uncommitted)
    pub fn load_diff(&mut self, branch_diff: bool) {
        let Some(selected) = self.table_state.selected() else {
            return;
        };
        let Some(agent) = self.agents.get(selected) else {
            return;
        };

        let path = &agent.path;
        let pane_id = agent.pane_id.clone();
        let worktree_name = self.extract_worktree_name(agent).0;

        let (diff_arg, title) = if branch_diff {
            // Get the base branch from git status if available, fallback to "main"
            let base = self
                .git_statuses
                .get(path)
                .map(|s| s.base_branch.as_str())
                .filter(|b| !b.is_empty())
                .unwrap_or("main");
            (
                format!("{}...HEAD", base),
                format!("Review: {} → {}", worktree_name, base),
            )
        } else {
            ("HEAD".to_string(), format!("WIP: {}", worktree_name))
        };

        // Include untracked files only for uncommitted changes view
        // Don't parse hunks eagerly - they're only needed for patch mode,
        // which reloads and parses them on demand via reload_unstaged_diff()
        let include_untracked = !branch_diff;
        let parse_hunks = false;
        match get_diff_content(path, &diff_arg, include_untracked, parse_hunks) {
            Ok((content, lines_added, lines_removed, hunks)) => {
                let (content, line_count) = if content.trim().is_empty() {
                    let msg = if branch_diff {
                        "No commits on this branch yet"
                    } else {
                        "No uncommitted changes"
                    };
                    (msg.to_string(), 1)
                } else {
                    let count = content.lines().count();
                    (content, count)
                };
                let parsed_lines = parse_ansi_to_lines(&content);

                // Get file list: from hunks for WIP, or via numstat for review mode
                let mut file_list = if !hunks.is_empty() {
                    extract_file_list(&hunks)
                } else {
                    get_file_list_numstat(path, &diff_arg, include_untracked)
                };
                map_file_offsets(&mut file_list, &parsed_lines);

                self.view_mode = ViewMode::Diff(Box::new(DiffView {
                    content,
                    parsed_lines,
                    scroll: 0,
                    line_count,
                    viewport_height: 0, // Will be set by UI
                    title,
                    worktree_path: path.clone(),
                    pane_id,
                    is_branch_diff: branch_diff,
                    lines_added,
                    lines_removed,
                    patch_mode: false,
                    hunks,
                    current_hunk: 0,
                    hunks_total: 0,
                    hunks_processed: 0,
                    staged_hunks: Vec::new(),
                    comment_input: None,
                    file_list,
                }));
            }
            Err(e) => {
                // Show error in diff view
                let parsed_lines = parse_ansi_to_lines(&e);
                self.view_mode = ViewMode::Diff(Box::new(DiffView {
                    content: e,
                    parsed_lines,
                    scroll: 0,
                    line_count: 1,
                    viewport_height: 0,
                    title: "Error".to_string(),
                    worktree_path: path.clone(),
                    pane_id,
                    is_branch_diff: branch_diff,
                    lines_added: 0,
                    lines_removed: 0,
                    patch_mode: false,
                    hunks: Vec::new(),
                    current_hunk: 0,
                    hunks_total: 0,
                    hunks_processed: 0,
                    staged_hunks: Vec::new(),
                    comment_input: None,
                    file_list: Vec::new(),
                }));
            }
        }
    }

    /// Close the diff modal and return to dashboard view
    pub fn close_diff(&mut self) {
        self.view_mode = ViewMode::Dashboard;
    }

    /// Send commit action to the agent pane and close diff modal
    pub fn send_commit_to_agent(&mut self) {
        if let ViewMode::Diff(diff) = &self.view_mode {
            let _ = tmux::send_keys(&diff.pane_id, self.config.dashboard.commit());
        }
        self.close_diff();
    }

    /// Send merge action to the agent pane and close diff modal
    pub fn trigger_merge(&mut self) {
        if let ViewMode::Diff(diff) = &self.view_mode {
            let _ = tmux::send_keys(&diff.pane_id, self.config.dashboard.merge());
        }
        self.close_diff();
    }

    /// Send commit action to the currently selected agent's pane (from dashboard view)
    pub fn send_commit_to_selected(&mut self) {
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            let _ = tmux::send_keys(&agent.pane_id, self.config.dashboard.commit());
        }
    }

    /// Send merge action to the currently selected agent's pane (from dashboard view)
    pub fn trigger_merge_for_selected(&mut self) {
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            let _ = tmux::send_keys(&agent.pane_id, self.config.dashboard.merge());
        }
    }
}
