use ansi_to_tui::IntoText;
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Cell, Paragraph, Row, Table, TableState},
};
use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cmd::Cmd;
use crate::config::Config;
use crate::tmux::{self, AgentPane};

/// Available sort modes for the agent list
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SortMode {
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

const TMUX_SORT_MODE_VAR: &str = "@workmux_sort_mode";

impl SortMode {
    /// Cycle to the next sort mode
    fn next(self) -> Self {
        match self {
            SortMode::Priority => SortMode::Project,
            SortMode::Project => SortMode::Recency,
            SortMode::Recency => SortMode::Natural,
            SortMode::Natural => SortMode::Priority,
        }
    }

    /// Get the display name for the sort mode
    fn label(&self) -> &'static str {
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
    fn load_from_tmux() -> Self {
        Cmd::new("tmux")
            .args(&["show-option", "-gqv", TMUX_SORT_MODE_VAR])
            .run_and_capture_stdout()
            .ok()
            .filter(|s| !s.is_empty())
            .map(|s| Self::from_str(&s))
            .unwrap_or_default()
    }

    /// Save sort mode to tmux global variable
    fn save_to_tmux(&self) {
        let _ = Cmd::new("tmux")
            .args(&["set-option", "-g", TMUX_SORT_MODE_VAR, self.as_str()])
            .run();
    }
}

/// Number of lines to capture from the agent's terminal for preview (scrollable history)
const PREVIEW_LINES: u16 = 200;

/// App state for the TUI
struct App {
    agents: Vec<AgentPane>,
    table_state: TableState,
    stale_threshold_secs: u64,
    config: Config,
    should_quit: bool,
    should_jump: bool,
    sort_mode: SortMode,
    /// Cached preview of the currently selected agent's terminal output
    preview: Option<String>,
    /// Track which pane_id the preview was captured from (to detect selection changes)
    preview_pane_id: Option<String>,
    /// Input mode: keystrokes are sent directly to the selected agent's pane
    input_mode: bool,
    /// Manual scroll offset for the preview (None = auto-scroll to bottom)
    preview_scroll: Option<u16>,
    /// Number of lines in the current preview content
    preview_line_count: u16,
    /// Height of the preview area (updated during rendering)
    preview_height: u16,
}

impl App {
    fn new() -> Result<Self> {
        let config = Config::load(None)?;
        let mut app = Self {
            agents: Vec::new(),
            table_state: TableState::default(),
            stale_threshold_secs: 60 * 60, // 60 minutes
            config,
            should_quit: false,
            should_jump: false,
            sort_mode: SortMode::load_from_tmux(),
            preview: None,
            preview_pane_id: None,
            input_mode: false,
            preview_scroll: None,
            preview_line_count: 0,
            preview_height: 0,
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

    fn refresh(&mut self) {
        self.agents = tmux::get_all_agent_panes().unwrap_or_default();
        self.sort_agents();

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

    /// Update the preview for the currently selected agent.
    /// Only fetches if the selection has changed or preview is stale.
    fn update_preview(&mut self) {
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
    fn refresh_preview(&mut self) {
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
    fn cycle_sort_mode(&mut self) {
        self.sort_mode = self.sort_mode.next();
        self.sort_mode.save_to_tmux();
        self.sort_agents();
    }

    fn next(&mut self) {
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

    fn previous(&mut self) {
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

    fn jump_to_selected(&mut self) {
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            self.should_jump = true;
            // Jump to the specific pane
            let _ = tmux::switch_to_pane(&agent.pane_id);
        }
    }

    fn jump_to_index(&mut self, index: usize) {
        if index < self.agents.len() {
            self.table_state.select(Some(index));
            self.jump_to_selected();
        }
    }

    fn peek_selected(&mut self) {
        // Switch to pane but keep popup open
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            let _ = tmux::switch_to_pane(&agent.pane_id);
            // Don't set should_jump - popup stays open
        }
    }

    /// Send a key to the selected agent's pane
    fn send_key_to_selected(&self, key: &str) {
        if let Some(selected) = self.table_state.selected()
            && let Some(agent) = self.agents.get(selected)
        {
            let _ = tmux::send_key(&agent.pane_id, key);
        }
    }

    /// Scroll preview up (toward older content). Returns the amount to scroll by.
    fn scroll_preview_up(&mut self, visible_height: u16, total_lines: u16) {
        let max_scroll = total_lines.saturating_sub(visible_height);
        let current = self.preview_scroll.unwrap_or(max_scroll);
        let half_page = visible_height / 2;
        self.preview_scroll = Some(current.saturating_sub(half_page));
    }

    /// Scroll preview down (toward newer content).
    fn scroll_preview_down(&mut self, visible_height: u16, total_lines: u16) {
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

    fn format_duration(&self, secs: u64) -> String {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let secs = secs % 60;
        format!("{:02}:{:02}:{:02}", hours, mins, secs)
    }

    fn is_stale(&self, agent: &AgentPane) -> bool {
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

    fn get_elapsed(&self, agent: &AgentPane) -> Option<u64> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        agent.status_ts.map(|ts| now.saturating_sub(ts))
    }

    fn get_status_display(&self, agent: &AgentPane) -> (String, Color) {
        let status = agent.status.as_deref().unwrap_or("");
        let is_stale = self.is_stale(agent);

        if is_stale {
            return ("stale".to_string(), Color::Gray);
        }

        // Match against configured icons
        let working = self.config.status_icons.working();
        let waiting = self.config.status_icons.waiting();
        let done = self.config.status_icons.done();

        if status == working {
            (status.to_string(), Color::Cyan)
        } else if status == waiting {
            (status.to_string(), Color::Magenta)
        } else if status == done {
            (status.to_string(), Color::Green)
        } else {
            (status.to_string(), Color::White)
        }
    }

    fn extract_agent_name(&self, agent: &AgentPane) -> String {
        // Try to extract a meaningful name from the window name
        // Remove common prefixes like "wm-"
        let name = &agent.window_name;
        let prefix = self.config.window_prefix();

        if let Some(stripped) = name.strip_prefix(prefix) {
            stripped.to_string()
        } else {
            // For non-workmux windows, show actual window name
            name.clone()
        }
    }

    fn extract_project_name(agent: &AgentPane) -> String {
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
}

pub fn run() -> Result<()> {
    // Check if tmux is running
    if !tmux::is_running().unwrap_or(false) {
        println!("No tmux server running.");
        return Ok(());
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    // Create app state
    let mut app = App::new()?;

    // Main loop
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = std::time::Instant::now();
    let refresh_interval = Duration::from_secs(2);
    let mut last_refresh = std::time::Instant::now();
    // Preview refreshes more frequently than the agent list
    // Use a faster refresh rate when in input mode for responsive typing feedback
    let preview_refresh_interval_normal = Duration::from_millis(500);
    let preview_refresh_interval_input = Duration::from_millis(100);
    let mut last_preview_refresh = std::time::Instant::now();

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        // Calculate timeout to respect the next scheduled preview refresh
        let current_preview_interval = if app.input_mode {
            preview_refresh_interval_input
        } else {
            preview_refresh_interval_normal
        };
        let time_until_preview =
            current_preview_interval.saturating_sub(last_preview_refresh.elapsed());
        let time_until_tick = tick_rate.saturating_sub(last_tick.elapsed());
        let timeout = time_until_tick.min(time_until_preview);

        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            if app.input_mode {
                // In input mode: forward keys to the selected pane
                match key.code {
                    KeyCode::Esc => {
                        app.input_mode = false;
                    }
                    KeyCode::Enter => {
                        app.send_key_to_selected("Enter");
                    }
                    KeyCode::Backspace => {
                        app.send_key_to_selected("BSpace");
                    }
                    KeyCode::Tab => {
                        app.send_key_to_selected("Tab");
                    }
                    KeyCode::Up => {
                        app.send_key_to_selected("Up");
                    }
                    KeyCode::Down => {
                        app.send_key_to_selected("Down");
                    }
                    KeyCode::Left => {
                        app.send_key_to_selected("Left");
                    }
                    KeyCode::Right => {
                        app.send_key_to_selected("Right");
                    }
                    KeyCode::Char(c) => {
                        // Send the character to the pane
                        app.send_key_to_selected(&c.to_string());
                    }
                    _ => {}
                }
                // Refresh preview immediately after sending input
                app.refresh_preview();
                last_preview_refresh = std::time::Instant::now();
            } else {
                // Normal mode: handle navigation and commands
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                    KeyCode::Char('j') | KeyCode::Down => app.next(),
                    KeyCode::Char('k') | KeyCode::Up => app.previous(),
                    KeyCode::Enter => app.jump_to_selected(),
                    KeyCode::Char('p') => app.peek_selected(),
                    KeyCode::Char('s') => app.cycle_sort_mode(),
                    KeyCode::Char('i') => {
                        // Enter input mode if an agent is selected
                        if app.table_state.selected().is_some() && !app.agents.is_empty() {
                            app.input_mode = true;
                        }
                    }
                    // Preview scrolling with Ctrl+U/D
                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.scroll_preview_up(app.preview_height, app.preview_line_count);
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.scroll_preview_down(app.preview_height, app.preview_line_count);
                    }
                    // Quick jump: 1-9 for rows 0-8
                    KeyCode::Char(c @ '1'..='9') => {
                        app.jump_to_index((c as u8 - b'1') as usize);
                    }
                    _ => {}
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
        }

        // Auto-refresh agent list every 2 seconds
        if last_refresh.elapsed() >= refresh_interval {
            app.refresh();
            last_refresh = std::time::Instant::now();
        }

        // Auto-refresh preview more frequently for live updates
        // Uses faster refresh rate in input mode (set at top of loop)
        if last_preview_refresh.elapsed() >= current_preview_interval {
            app.refresh_preview();
            last_preview_refresh = std::time::Instant::now();
        }

        if app.should_quit || app.should_jump {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn ui(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Layout: table (top), preview (bottom), footer
    let chunks = Layout::vertical([
        Constraint::Percentage(45), // Table (top half)
        Constraint::Min(5),         // Preview (bottom half, at least 5 lines)
        Constraint::Length(1),      // Footer
    ])
    .split(area);

    // Table
    render_table(f, app, chunks[0]);

    // Preview
    render_preview(f, app, chunks[1]);

    // Footer - show different help based on mode
    let footer_text = if app.input_mode {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  INPUT MODE",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Type to send keys to agent  "),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" exit"),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("  [i]", Style::default().fg(Color::Green)),
            Span::raw(" input  "),
            Span::styled("[1-9]", Style::default().fg(Color::Yellow)),
            Span::raw(" jump  "),
            Span::styled("[p]", Style::default().fg(Color::Cyan)),
            Span::raw(" peek  "),
            Span::styled("[s]", Style::default().fg(Color::Cyan)),
            Span::raw(" sort: "),
            Span::styled(app.sort_mode.label(), Style::default().fg(Color::Green)),
            Span::raw("  "),
            Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
            Span::raw(" go  "),
            Span::styled("[q]", Style::default().fg(Color::Cyan)),
            Span::raw(" quit"),
        ]))
    };
    f.render_widget(footer_text, chunks[1 + 1]);
}

fn render_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_cells = ["#", "Project", "Agent", "Status", "Time", "Title"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Cyan).bold()));
    let header = Row::new(header_cells).height(1);

    // Group agents by (session, window_name) to detect multi-pane windows
    let mut window_groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (idx, agent) in app.agents.iter().enumerate() {
        let key = (agent.session.clone(), agent.window_name.clone());
        window_groups.entry(key).or_default().push(idx);
    }

    // Build a set of windows with multiple panes
    let multi_pane_windows: std::collections::HashSet<(String, String)> = window_groups
        .iter()
        .filter(|(_, indices)| indices.len() > 1)
        .map(|(key, _)| key.clone())
        .collect();

    // Track position within each window group for pane numbering
    let mut window_positions: BTreeMap<(String, String), usize> = BTreeMap::new();

    // Pre-compute row data to calculate max widths
    let row_data: Vec<_> = app
        .agents
        .iter()
        .enumerate()
        .map(|(idx, agent)| {
            let key = (agent.session.clone(), agent.window_name.clone());
            let is_multi_pane = multi_pane_windows.contains(&key);

            let pane_suffix = if is_multi_pane {
                let pos = window_positions.entry(key.clone()).or_insert(0);
                *pos += 1;
                format!(" [{}]", pos)
            } else {
                String::new()
            };

            let jump_key = if idx < 9 {
                format!("{}", idx + 1)
            } else {
                String::new()
            };

            let project = App::extract_project_name(agent);
            let agent_name = format!("{}{}", app.extract_agent_name(agent), pane_suffix);
            let title = agent
                .pane_title
                .as_ref()
                .map(|t| t.strip_prefix("✳ ").unwrap_or(t).to_string())
                .unwrap_or_default();
            let (status_text, status_color) = app.get_status_display(agent);
            let duration = app
                .get_elapsed(agent)
                .map(|d| app.format_duration(d))
                .unwrap_or_else(|| "-".to_string());

            (
                jump_key,
                project,
                agent_name,
                status_text,
                status_color,
                duration,
                title,
            )
        })
        .collect();

    // Calculate max project name width (with padding, capped)
    let max_project_width = row_data
        .iter()
        .map(|(_, project, _, _, _, _, _)| project.len())
        .max()
        .unwrap_or(5)
        .clamp(5, 20) // min 5, max 20
        + 2; // padding

    // Calculate max agent name width (with padding, capped)
    let max_agent_width = row_data
        .iter()
        .map(|(_, _, agent_name, _, _, _, _)| agent_name.len())
        .max()
        .unwrap_or(5)
        .clamp(5, 24) // min 5, max 24
        + 2; // padding

    let rows: Vec<Row> = row_data
        .into_iter()
        .map(
            |(jump_key, project, agent_name, status_text, status_color, duration, title)| {
                Row::new(vec![
                    Cell::from(jump_key).style(Style::default().fg(Color::Yellow)),
                    Cell::from(project),
                    Cell::from(agent_name),
                    Cell::from(status_text).style(Style::default().fg(status_color)),
                    Cell::from(duration),
                    Cell::from(title),
                ])
            },
        )
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),                        // #: jump key
            Constraint::Length(max_project_width as u16), // Project: auto-sized
            Constraint::Length(max_agent_width as u16),   // Agent: auto-sized
            Constraint::Length(8),                        // Status: fixed (icons)
            Constraint::Length(10),                       // Time: HH:MM:SS + padding
            Constraint::Fill(1),                          // Title: takes remaining space
        ],
    )
    .header(header)
    .block(Block::default())
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("> ");

    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_preview(f: &mut Frame, app: &mut App, area: Rect) {
    // Get info about the selected agent for the title
    let selected_agent = app
        .table_state
        .selected()
        .and_then(|idx| app.agents.get(idx));

    let (title, title_style, border_style) = if app.input_mode {
        let agent_name = selected_agent
            .map(|a| app.extract_agent_name(a))
            .unwrap_or_default();
        (
            format!(" INPUT: {} ", agent_name),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::Green),
        )
    } else if let Some(agent) = selected_agent {
        let agent_name = app.extract_agent_name(agent);
        (
            format!(" Preview: {} ", agent_name),
            Style::default().fg(Color::Cyan),
            Style::default().fg(Color::DarkGray),
        )
    } else {
        (
            " Preview ".to_string(),
            Style::default().fg(Color::Cyan),
            Style::default().fg(Color::DarkGray),
        )
    };

    let block = Block::bordered()
        .title(title)
        .title_style(title_style)
        .border_style(border_style);

    // Calculate the inner area to determine scroll offset
    let inner_area = block.inner(area);

    // Update preview height for scroll calculations
    app.preview_height = inner_area.height;

    // Get preview content or show placeholder
    let (text, line_count) = match (&app.preview, selected_agent) {
        (Some(preview), Some(_)) => {
            let trimmed = preview.trim_end();
            if trimmed.is_empty() {
                (Text::raw("(empty output)"), 1u16)
            } else {
                // Parse ANSI escape sequences to get colored text
                match trimmed.into_text() {
                    Ok(text) => {
                        let count = text.lines.len() as u16;
                        (text, count)
                    }
                    Err(_) => {
                        // Fallback to plain text if ANSI parsing fails
                        let count = trimmed.lines().count() as u16;
                        (Text::raw(trimmed), count)
                    }
                }
            }
        }
        (None, Some(_)) => (Text::raw("(pane not available)"), 1),
        (_, None) => (Text::raw("(no agent selected)"), 1),
    };

    // Update line count for scroll calculations
    app.preview_line_count = line_count;

    // Calculate scroll offset: use manual scroll if set, otherwise auto-scroll to bottom
    let max_scroll = line_count.saturating_sub(inner_area.height);
    let scroll_offset = app.preview_scroll.unwrap_or(max_scroll);

    let paragraph = Paragraph::new(text).block(block).scroll((scroll_offset, 0));

    f.render_widget(paragraph, area);
}
