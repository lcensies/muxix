//! Sidebar TUI for monitoring active workmux agents.
//!
//! Provides a compact, always-visible agent status list in a narrow tmux pane.
//! Currently tmux-only. The sidebar is toggled via `workmux sidebar` and
//! rendered by the hidden `workmux _sidebar-run` command.
//!
//! When enabled, a sidebar pane is created in every existing tmux window.
//! A tmux hook ensures new windows also get a sidebar automatically.

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

/// Toggle the sidebar globally across all tmux windows.
pub fn toggle(width: Option<u16>) -> Result<()> {
    let width = width.unwrap_or(DEFAULT_WIDTH).max(10);

    if std::env::var("TMUX").is_err() {
        return Err(anyhow!("Sidebar requires tmux"));
    }

    // Check if sidebar is currently enabled
    if is_sidebar_enabled() {
        // Toggling OFF: kill all sidebar panes, remove hooks, unset options
        kill_all_sidebars();
        remove_hooks();
        let _ = Cmd::new("tmux")
            .args(&["set-option", "-gu", "@workmux_sidebar_enabled"])
            .run();
        let _ = Cmd::new("tmux")
            .args(&["set-option", "-gu", "@workmux_sidebar_width"])
            .run();
        return Ok(());
    }

    // Toggling ON: set global options, create sidebars in all windows, install hooks
    let width_str = width.to_string();
    Cmd::new("tmux")
        .args(&["set-option", "-g", "@workmux_sidebar_enabled", "1"])
        .run()?;
    Cmd::new("tmux")
        .args(&["set-option", "-g", "@workmux_sidebar_width", &width_str])
        .run()?;

    create_sidebars_in_all_windows(width)?;
    install_hooks()?;

    Ok(())
}

/// Sync sidebar into the current window (called by tmux hooks for new windows).
pub fn sync() -> Result<()> {
    if !is_sidebar_enabled() {
        return Ok(());
    }

    let width = sidebar_width();

    // Check if current window already has a sidebar
    if find_sidebar_in_current_window()?.is_some() {
        return Ok(());
    }

    // Create sidebar in current window
    create_sidebar_in_current_window(width)?;

    Ok(())
}

fn is_sidebar_enabled() -> bool {
    Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@workmux_sidebar_enabled"])
        .run_and_capture_stdout()
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

fn sidebar_width() -> u16 {
    Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@workmux_sidebar_width"])
        .run_and_capture_stdout()
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(DEFAULT_WIDTH)
}

/// Find a sidebar pane in the current tmux window.
fn find_sidebar_in_current_window() -> Result<Option<String>> {
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

/// Create a sidebar pane in the current window.
fn create_sidebar_in_current_window(width: u16) -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_str().ok_or_else(|| anyhow!("exe path not UTF-8"))?;
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

/// Create sidebars in all existing tmux windows.
fn create_sidebars_in_all_windows(width: u16) -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_str().ok_or_else(|| anyhow!("exe path not UTF-8"))?;
    let width_str = width.to_string();

    // Get all window IDs
    let output = Cmd::new("tmux")
        .args(&["list-windows", "-a", "-F", "#{window_id}"])
        .run_and_capture_stdout()?;

    for window_id in output.lines() {
        let window_id = window_id.trim();
        if window_id.is_empty() {
            continue;
        }

        // Check if this window already has a sidebar
        let panes = Cmd::new("tmux")
            .args(&["list-panes", "-t", window_id, "-F", "#{@workmux_role}"])
            .run_and_capture_stdout()
            .unwrap_or_default();

        if panes.lines().any(|l| l.trim() == SIDEBAR_ROLE_VALUE) {
            continue;
        }

        // Get the first pane in the window as split target
        let target = Cmd::new("tmux")
            .args(&["list-panes", "-t", window_id, "-F", "#{pane_id}"])
            .run_and_capture_stdout()
            .ok()
            .and_then(|s| s.lines().next().map(|l| l.trim().to_string()));

        let Some(target_pane) = target else {
            continue;
        };

        let new_pane_id = Cmd::new("tmux")
            .args(&[
                "split-window",
                "-hbf",
                "-l",
                &width_str,
                "-d",
                "-t",
                &target_pane,
                "-P",
                "-F",
                "#{pane_id}",
                exe_str,
                "_sidebar-run",
            ])
            .run_and_capture_stdout();

        if let Ok(pane_id) = new_pane_id {
            let pane_id = pane_id.trim();
            let _ = Cmd::new("tmux")
                .args(&[
                    "set-option",
                    "-p",
                    "-t",
                    pane_id,
                    "@workmux_role",
                    SIDEBAR_ROLE_VALUE,
                ])
                .run();
        }
    }

    Ok(())
}

/// Kill all sidebar panes across all windows.
fn kill_all_sidebars() {
    let output = Cmd::new("tmux")
        .args(&["list-panes", "-a", "-F", "#{pane_id} #{@workmux_role}"])
        .run_and_capture_stdout()
        .unwrap_or_default();

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 && parts[1].trim() == SIDEBAR_ROLE_VALUE {
            let _ = Cmd::new("tmux").args(&["kill-pane", "-t", parts[0]]).run();
        }
    }
}

/// Install tmux hooks so new windows automatically get a sidebar.
fn install_hooks() -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_str().ok_or_else(|| anyhow!("exe path not UTF-8"))?;

    let sync_cmd = format!("run-shell -b '{} _sidebar-sync'", exe_str);

    Cmd::new("tmux")
        .args(&[
            "set-hook",
            "-g",
            "after-new-window[workmux_sidebar]",
            &sync_cmd,
        ])
        .run()?;

    Ok(())
}

/// Remove tmux hooks.
fn remove_hooks() {
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "after-new-window[workmux_sidebar]"])
        .run();
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
