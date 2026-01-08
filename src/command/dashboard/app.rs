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

use super::sort::SortMode;

/// Strip ANSI escape sequences from a string
fn strip_ansi_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Skip until we hit a letter (the terminator)
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Number of lines to capture from the agent's terminal for preview (scrollable history)
pub const PREVIEW_LINES: u16 = 200;

/// Current view mode of the dashboard
#[derive(Debug, Default, PartialEq)]
pub enum ViewMode {
    #[default]
    Dashboard,
    Diff(DiffView),
}

/// State for the diff view
#[derive(Debug, PartialEq)]
pub struct DiffView {
    /// The diff content (with ANSI colors)
    pub content: String,
    /// Current scroll offset (use usize to handle large diffs)
    pub scroll: usize,
    /// Total line count for scroll bounds
    pub line_count: usize,
    /// Viewport height (updated by UI during render for page scroll)
    pub viewport_height: u16,
    /// Title for the view (e.g., "WIP: fix-bug")
    pub title: String,
    /// Path to the worktree (for commit/merge actions)
    pub worktree_path: PathBuf,
    /// Pane ID for sending commands to agent
    pub pane_id: String,
    /// Whether this is a branch diff (true) or uncommitted diff (false)
    pub is_branch_diff: bool,
    /// Number of lines added in the diff
    pub lines_added: usize,
    /// Number of lines removed in the diff
    pub lines_removed: usize,
}

impl DiffView {
    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        let max_scroll = self
            .line_count
            .saturating_sub(self.viewport_height as usize);
        if self.scroll < max_scroll {
            self.scroll += 1;
        }
    }

    pub fn scroll_page_up(&mut self) {
        let page = self.viewport_height as usize;
        self.scroll = self.scroll.saturating_sub(page);
    }

    pub fn scroll_page_down(&mut self) {
        let page = self.viewport_height as usize;
        let max_scroll = self
            .line_count
            .saturating_sub(self.viewport_height as usize);
        self.scroll = (self.scroll + page).min(max_scroll);
    }
}

/// App state for the TUI
pub struct App {
    pub agents: Vec<AgentPane>,
    pub table_state: TableState,
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
    is_git_fetching: Arc<AtomicBool>,
    /// Frame counter for spinner animation (increments each tick)
    pub spinner_frame: u8,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = Config::load(None)?;
        let (git_tx, git_rx) = mpsc::channel();
        let mut app = Self {
            agents: Vec::new(),
            table_state: TableState::default(),
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
        };
        app.refresh();
        // Select first item if available
        if !app.agents.is_empty() {
            app.table_state.select(Some(0));
        }
        // Initial preview fetch
        app.update_preview();
        Ok(app)
    }

    pub fn refresh(&mut self) {
        self.agents = tmux::get_all_agent_panes().unwrap_or_default();
        self.sort_agents();

        // Consume any pending git status updates from background thread
        while let Ok((path, status)) = self.git_rx.try_recv() {
            self.git_statuses.insert(path, status);
        }

        // Trigger background git status fetch every 5 seconds
        if self.last_git_fetch.elapsed() >= Duration::from_secs(5) {
            self.last_git_fetch = std::time::Instant::now();
            self.spawn_git_status_fetch();
        }

        // Adjust selection if it's now out of bounds
        if let Some(selected) = self.table_state.selected()
            && selected >= self.agents.len()
        {
            self.table_state.select(if self.agents.is_empty() {
                None
            } else {
                Some(self.agents.len() - 1)
            });
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
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let secs = secs % 60;
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    }

    pub fn is_stale(&self, agent: &AgentPane) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        if let Some(ts) = agent.status_ts {
            now.saturating_sub(ts) > self.stale_threshold_secs
        } else {
            false
        }
    }

    pub fn get_elapsed(&self, agent: &AgentPane) -> Option<u64> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        agent.status_ts.map(|ts| now.saturating_sub(ts))
    }

    pub fn get_status_display(&self, agent: &AgentPane) -> (String, Color) {
        let status = agent.status.as_deref().unwrap_or("");
        let is_stale = self.is_stale(agent);

        // Match against configured icons
        let working = self.config.status_icons.working();
        let waiting = self.config.status_icons.waiting();
        let done = self.config.status_icons.done();

        // Get the base status text and color
        let (status_text, base_color) = if status == working {
            (status.to_string(), Color::Cyan)
        } else if status == waiting {
            (status.to_string(), Color::Magenta)
        } else if status == done {
            (status.to_string(), Color::Green)
        } else {
            (status.to_string(), Color::White)
        };

        // If stale, dim the color and add timer-off indicator
        if is_stale {
            let display_text = format!("{} \u{f051b}", status_text);
            (display_text, Color::DarkGray)
        } else {
            (status_text, base_color)
        }
    }

    /// Extract the worktree name from an agent.
    /// Returns (worktree_name, is_main) where is_main indicates if this is the main worktree.
    pub fn extract_worktree_name(&self, agent: &AgentPane) -> (String, bool) {
        let name = &agent.window_name;
        let prefix = self.config.window_prefix();

        if let Some(stripped) = name.strip_prefix(prefix) {
            // Workmux-created worktree agent
            (stripped.to_string(), false)
        } else {
            // Non-workmux agent - running in main worktree
            ("main".to_string(), true)
        }
    }

    pub fn extract_project_name(agent: &AgentPane) -> String {
        // Extract project name from the path
        // Look for __worktrees pattern or use directory name
        let path = &agent.path;

        // Walk up the path to find __worktrees
        for ancestor in path.ancestors() {
            if let Some(name) = ancestor.file_name() {
                let name_str = name.to_string_lossy();
                if name_str.ends_with("__worktrees") {
                    // Return the project name (part before __worktrees)
                    return name_str
                        .strip_suffix("__worktrees")
                        .unwrap_or(&name_str)
                        .to_string();
                }
            }
        }

        // Fallback: use the directory name (for non-worktree projects)
        path.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string())
    }

    /// Check if delta pager is available
    fn has_delta() -> bool {
        std::process::Command::new("which")
            .arg("delta")
            .output()
            .is_ok_and(|o| o.status.success())
    }

    /// Count added and removed lines from raw diff content
    fn count_diff_stats(content: &[u8]) -> (usize, usize) {
        let text = String::from_utf8_lossy(content);
        let mut added = 0;
        let mut removed = 0;
        for line in text.lines() {
            // Strip ANSI escape sequences for reliable matching
            let stripped = strip_ansi_escapes(line);
            if stripped.starts_with('+') && !stripped.starts_with("+++") {
                added += 1;
            } else if stripped.starts_with('-') && !stripped.starts_with("---") {
                removed += 1;
            }
        }
        (added, removed)
    }

    /// Get diff content, optionally piped through delta for syntax highlighting
    /// Returns (content, lines_added, lines_removed)
    fn get_diff_content(
        path: &PathBuf,
        diff_arg: &str,
        include_untracked: bool,
    ) -> Result<(String, usize, usize), String> {
        // Run git diff
        let git_output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("--no-pager")
            .arg("diff")
            .arg("--color=always")
            .arg(diff_arg)
            .output()
            .map_err(|e| format!("Error running git diff: {}", e))?;

        let mut diff_content = git_output.stdout;

        // For uncommitted changes, also include untracked files
        if include_untracked {
            let untracked_diff = Self::get_untracked_files_diff(path)?;
            if !untracked_diff.is_empty() {
                diff_content.extend_from_slice(untracked_diff.as_bytes());
            }
        }

        // Count stats before any transformation
        let (lines_added, lines_removed) = Self::count_diff_stats(&diff_content);

        // If empty or delta not available, return as-is
        if diff_content.is_empty() || !Self::has_delta() {
            return Ok((
                String::from_utf8_lossy(&diff_content).to_string(),
                lines_added,
                lines_removed,
            ));
        }

        // Pipe through delta for syntax highlighting
        let mut delta = std::process::Command::new("delta")
            .arg("--paging=never")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("Error running delta: {}", e))?;

        // Write git diff output to delta's stdin
        if let Some(mut stdin) = delta.stdin.take() {
            use std::io::Write;
            let _ = stdin.write_all(&diff_content);
        }

        let delta_output = delta
            .wait_with_output()
            .map_err(|e| format!("Error reading delta output: {}", e))?;

        Ok((
            String::from_utf8_lossy(&delta_output.stdout).to_string(),
            lines_added,
            lines_removed,
        ))
    }

    /// Generate diff output for untracked files (new files not yet staged)
    fn get_untracked_files_diff(path: &PathBuf) -> Result<String, String> {
        // Get list of untracked files
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(path)
            .arg("ls-files")
            .arg("--others")
            .arg("--exclude-standard")
            .output()
            .map_err(|e| format!("Error listing untracked files: {}", e))?;

        let output_str = String::from_utf8_lossy(&output.stdout).to_string();
        let untracked_files: Vec<&str> = output_str.lines().filter(|l| !l.is_empty()).collect();

        if untracked_files.is_empty() {
            return Ok(String::new());
        }

        // Generate diff for each untracked file using git diff --no-index
        let mut result = String::new();
        for file in untracked_files {
            let file_path = path.join(file);
            if !file_path.is_file() {
                continue;
            }

            // Use git diff --no-index to generate proper diff format for new files
            let diff_output = std::process::Command::new("git")
                .arg("-C")
                .arg(path)
                .arg("diff")
                .arg("--no-index")
                .arg("--color=always")
                .arg("/dev/null")
                .arg(file)
                .output();

            if let Ok(output) = diff_output {
                // git diff --no-index returns exit code 1 when files differ, which is expected
                let diff_text = String::from_utf8_lossy(&output.stdout);
                if !diff_text.is_empty() {
                    result.push_str(&diff_text);
                }
            }
        }

        Ok(result)
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
        let include_untracked = !branch_diff;
        match Self::get_diff_content(path, &diff_arg, include_untracked) {
            Ok((content, lines_added, lines_removed)) => {
                let (content, line_count) = if content.trim().is_empty() {
                    ("No changes".to_string(), 1)
                } else {
                    let count = content.lines().count();
                    (content, count)
                };

                self.view_mode = ViewMode::Diff(DiffView {
                    content,
                    scroll: 0,
                    line_count,
                    viewport_height: 0, // Will be set by UI
                    title,
                    worktree_path: path.clone(),
                    pane_id,
                    is_branch_diff: branch_diff,
                    lines_added,
                    lines_removed,
                });
            }
            Err(e) => {
                // Show error in diff view
                self.view_mode = ViewMode::Diff(DiffView {
                    content: e,
                    scroll: 0,
                    line_count: 1,
                    viewport_height: 0,
                    title: "Error".to_string(),
                    worktree_path: path.clone(),
                    pane_id,
                    is_branch_diff: branch_diff,
                    lines_added: 0,
                    lines_removed: 0,
                });
            }
        }
    }

    /// Close the diff modal and return to dashboard view
    pub fn close_diff(&mut self) {
        self.view_mode = ViewMode::Dashboard;
    }

    /// Send commit command to the agent pane and close diff modal
    pub fn send_commit_to_agent(&mut self) {
        if let ViewMode::Diff(diff) = &self.view_mode {
            // Send /commit command to the agent's pane
            // Note: This assumes the agent is ready to receive input
            let _ = tmux::send_keys(&diff.pane_id, "/commit\n");
        }
        self.close_diff();
    }

    /// Trigger merge workflow and close diff modal
    pub fn trigger_merge(&mut self) {
        if let ViewMode::Diff(diff) = &self.view_mode {
            // Run workmux merge in the worktree directory
            let _ = std::process::Command::new("workmux")
                .arg("merge")
                .current_dir(&diff.worktree_path)
                .spawn();
        }
        self.close_diff();
        self.should_quit = true; // Exit dashboard after merge
    }
}
