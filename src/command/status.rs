use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Cell, Paragraph, Row, Table, TableState},
};
use std::collections::BTreeMap;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::tmux::{self, AgentPane};

/// App state for the TUI
struct App {
    agents: Vec<AgentPane>,
    table_state: TableState,
    stale_threshold_secs: u64,
    config: Config,
    should_quit: bool,
    should_jump: bool,
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
        };
        app.refresh();
        // Select first item if available
        if !app.agents.is_empty() {
            app.table_state.select(Some(0));
        }
        Ok(app)
    }

    fn refresh(&mut self) {
        // Natural tmux order (by pane_id) for stable positions
        self.agents = tmux::get_all_agent_panes().unwrap_or_default();

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

    fn extract_project_name(&self, agent: &AgentPane) -> String {
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

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                KeyCode::Char('j') | KeyCode::Down => app.next(),
                KeyCode::Char('k') | KeyCode::Up => app.previous(),
                KeyCode::Enter => app.jump_to_selected(),
                KeyCode::Char('p') => app.peek_selected(),
                // Quick jump: 1-9 for rows 0-8
                KeyCode::Char(c @ '1'..='9') => {
                    app.jump_to_index((c as u8 - b'1') as usize);
                }
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
        }

        // Auto-refresh every 2 seconds
        if last_refresh.elapsed() >= refresh_interval {
            app.refresh();
            last_refresh = std::time::Instant::now();
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

    // Layout: table, footer
    let chunks = Layout::vertical([
        Constraint::Min(5),    // Table
        Constraint::Length(1), // Footer
    ])
    .split(area);

    // Table
    render_table(f, app, chunks[0]);

    // Footer
    let footer_text = Paragraph::new(Line::from(vec![
        Span::styled("  [1-9]", Style::default().fg(Color::Yellow)),
        Span::raw(" jump  "),
        Span::styled("[p]", Style::default().fg(Color::Cyan)),
        Span::raw(" peek  "),
        Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
        Span::raw(" go  "),
        Span::styled("[q]", Style::default().fg(Color::Cyan)),
        Span::raw(" quit"),
    ]));
    f.render_widget(footer_text, chunks[1]);
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

    let rows: Vec<Row> = app
        .agents
        .iter()
        .enumerate()
        .map(|(idx, agent)| {
            let key = (agent.session.clone(), agent.window_name.clone());
            let is_multi_pane = multi_pane_windows.contains(&key);

            // Add pane number suffix for multi-pane windows
            let pane_suffix = if is_multi_pane {
                let pos = window_positions.entry(key.clone()).or_insert(0);
                *pos += 1;
                format!(" [{}]", pos)
            } else {
                String::new()
            };

            // Quick jump key: 1-9 for first 9 rows
            let jump_key = if idx < 9 {
                format!("{}", idx + 1)
            } else {
                String::new()
            };

            let project = app.extract_project_name(agent);
            let agent_name = format!("{}{}", app.extract_agent_name(agent), pane_suffix);
            // Extract pane title (Claude Code session summary), strip leading "✳ " if present
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

            Row::new(vec![
                Cell::from(jump_key).style(Style::default().fg(Color::Yellow)),
                Cell::from(project),
                Cell::from(agent_name),
                Cell::from(status_text).style(Style::default().fg(status_color)),
                Cell::from(duration),
                Cell::from(title),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),  // #: jump key
            Constraint::Max(20),    // Project: cap width
            Constraint::Max(24),    // Agent: cap width
            Constraint::Length(8),  // Status: fixed (icons)
            Constraint::Length(10), // Time: HH:MM:SS + padding
            Constraint::Fill(1),    // Title: takes remaining space
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
