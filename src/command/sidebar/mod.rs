//! Sidebar TUI for monitoring active workmux agents.
//!
//! Provides a compact, always-visible agent status list in a narrow tmux pane.
//! Currently tmux-only. The sidebar is toggled via `workmux sidebar` and
//! rendered by the hidden `workmux _sidebar-run` command.

mod app;
mod ui;

use anyhow::{Result, anyhow};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::backend::CrosstermBackend;
use std::io;
use std::time::Duration;

use crate::cmd::Cmd;
use crate::multiplexer::{create_backend, detect_backend};

use self::app::SidebarApp;
use self::ui::render_sidebar;

const SIDEBAR_ROLE_VALUE: &str = "sidebar";
const DEFAULT_WIDTH: u16 = 30;

/// Toggle the sidebar in the current tmux window.
pub fn toggle(width: Option<u16>) -> Result<()> {
    let width = width.unwrap_or(DEFAULT_WIDTH).max(10);

    // Ensure we're in tmux
    if std::env::var("TMUX").is_err() {
        return Err(anyhow!("Sidebar requires tmux"));
    }

    // Check if sidebar already exists in current window
    if let Some(sidebar_pane_id) = find_sidebar_pane()? {
        // Kill existing sidebar
        let _ = Cmd::new("tmux")
            .args(&["kill-pane", "-t", &sidebar_pane_id])
            .run();
        return Ok(());
    }

    // Get the workmux binary path
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_str().ok_or_else(|| anyhow!("exe path not UTF-8"))?;

    // Create left-side split: -h (horizontal), -b (before), -f (full height), -d (don't focus)
    let width_str = width.to_string();
    let new_pane_id = Cmd::new("tmux")
        .args(&[
            "split-window",
            "-hbf",
            "-l",
            &width_str,
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            exe_str,
            "_sidebar-run",
        ])
        .run_and_capture_stdout()?
        .trim()
        .to_string();

    // Tag the pane so we can find it later
    Cmd::new("tmux")
        .args(&[
            "set-option",
            "-p",
            "-t",
            &new_pane_id,
            "@workmux_role",
            SIDEBAR_ROLE_VALUE,
        ])
        .run()?;

    Ok(())
}

/// Find the sidebar pane in the current tmux window.
/// Returns the pane_id if found and alive.
fn find_sidebar_pane() -> Result<Option<String>> {
    // Query all panes in current window with their @workmux_role option
    let output = Cmd::new("tmux")
        .args(&["list-panes", "-F", "#{pane_id} #{@workmux_role}"])
        .run_and_capture_stdout()?;

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 && parts[1].trim() == SIDEBAR_ROLE_VALUE {
            return Ok(Some(parts[0].to_string()));
        }
    }

    Ok(None)
}

/// Drop guard that restores terminal state on panic or early return.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// Run the sidebar TUI (called by the hidden `_sidebar-run` command).
pub fn run_sidebar() -> Result<()> {
    let mux = create_backend(detect_backend());

    if !mux.is_running().unwrap_or(false) {
        return Ok(());
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Drop guard ensures terminal is restored even on panic/error
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let mut app = SidebarApp::new(mux)?;

    // Main loop
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = std::time::Instant::now();
    let refresh_interval = Duration::from_secs(2);
    let mut last_refresh = std::time::Instant::now();

    loop {
        terminal.draw(|f| render_sidebar(f, &mut app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());

        if event::poll(timeout)? {
            let event = event::read()?;

            let Event::Key(key) = event else { continue };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    app.should_quit = true;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    app.next();
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    app.previous();
                }
                KeyCode::Enter => {
                    app.jump_to_selected();
                }
                KeyCode::Char('G') => {
                    app.select_last();
                }
                KeyCode::Char('g') => {
                    app.select_first();
                }
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
            app.tick();
        }

        // Auto-refresh agent list
        if last_refresh.elapsed() >= refresh_interval {
            app.refresh();
            last_refresh = std::time::Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    // _guard handles cleanup on drop (including the normal exit path)
    Ok(())
}
