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
const MIN_WIDTH: u16 = 25;
const MAX_WIDTH: u16 = 50;

/// Compute sidebar width as ~10% of terminal width, clamped to [MIN_WIDTH, MAX_WIDTH].
fn default_width() -> u16 {
    let client_width: u16 = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{client_width}"])
        .run_and_capture_stdout()
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);

    if client_width == 0 {
        return MIN_WIDTH;
    }

    (client_width * 10 / 100).clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Toggle the sidebar globally across all tmux windows.
pub fn toggle(width: Option<u16>) -> Result<()> {
    let width = width.unwrap_or_else(default_width).max(10);

    if std::env::var("TMUX").is_err() {
        return Err(anyhow!("Sidebar requires tmux"));
    }

    // Determine intent based on the current window's state
    let current_window = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{window_id}"])
        .run_and_capture_stdout()?
        .trim()
        .to_string();

    let current_has_sidebar = find_sidebar_in_window(&current_window).unwrap_or(false);

    if current_has_sidebar {
        // Current window has sidebar → toggle OFF globally
        kill_all_sidebars_and_restore_layouts();
        remove_hooks();
        let _ = Cmd::new("tmux")
            .args(&["set-option", "-gu", "@workmux_sidebar_enabled"])
            .run();
        let _ = Cmd::new("tmux")
            .args(&["set-option", "-gu", "@workmux_sidebar_width"])
            .run();
        return Ok(());
    }

    // Current window missing sidebar → enable/repair globally
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

/// Sync sidebar into a window (called by tmux hooks for new windows/sessions).
pub fn sync(window_id: Option<&str>) -> Result<()> {
    if !is_sidebar_enabled() {
        return Ok(());
    }

    let width = sidebar_width();

    // Use the provided window ID or fall back to current window
    let target = match window_id {
        Some(id) => id.to_string(),
        None => Cmd::new("tmux")
            .args(&["display-message", "-p", "#{window_id}"])
            .run_and_capture_stdout()?
            .trim()
            .to_string(),
    };

    if target.is_empty() {
        return Ok(());
    }

    // Check if this window already has a sidebar
    if find_sidebar_in_window(&target)? {
        return Ok(());
    }

    // Create sidebar in the target window
    create_sidebar_in_window(&target, width)?;

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
        .unwrap_or_else(default_width)
}

/// Check if a window already has a sidebar pane.
fn find_sidebar_in_window(window_id: &str) -> Result<bool> {
    let output = Cmd::new("tmux")
        .args(&["list-panes", "-t", window_id, "-F", "#{@workmux_role}"])
        .run_and_capture_stdout()?;

    Ok(output.lines().any(|l| l.trim() == SIDEBAR_ROLE_VALUE))
}

/// Create a sidebar pane in a specific window (idempotent).
fn create_sidebar_in_window(window_id: &str, width: u16) -> Result<()> {
    if find_sidebar_in_window(window_id).unwrap_or(false) {
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let exe_str = exe.to_str().ok_or_else(|| anyhow!("exe path not UTF-8"))?;
    let width_str = width.to_string();

    save_window_layout(window_id);

    // Get the first pane in the window as split target
    let target_pane = Cmd::new("tmux")
        .args(&["list-panes", "-t", window_id, "-F", "#{pane_id}"])
        .run_and_capture_stdout()?;
    let target_pane = target_pane.lines().next().map(|l| l.trim()).unwrap_or("");
    if target_pane.is_empty() {
        return Ok(());
    }

    let new_pane_id = Cmd::new("tmux")
        .args(&[
            "split-window",
            "-hbf",
            "-l",
            &width_str,
            "-t",
            target_pane,
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
    let output = Cmd::new("tmux")
        .args(&["list-windows", "-a", "-F", "#{window_id}"])
        .run_and_capture_stdout()?;

    for window_id in output.lines() {
        let window_id = window_id.trim();
        if window_id.is_empty() {
            continue;
        }
        let _ = create_sidebar_in_window(window_id, width);
    }

    Ok(())
}

/// Kill all sidebar panes and restore the original layout in each window.
fn kill_all_sidebars_and_restore_layouts() {
    // Find all sidebar panes with their window IDs
    let output = Cmd::new("tmux")
        .args(&[
            "list-panes",
            "-a",
            "-F",
            "#{window_id} #{pane_id} #{@workmux_role}",
        ])
        .run_and_capture_stdout()
        .unwrap_or_default();

    let mut windows_with_sidebars = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() == 3 && parts[2].trim() == SIDEBAR_ROLE_VALUE {
            windows_with_sidebars.push(parts[0].to_string());
            let _ = Cmd::new("tmux").args(&["kill-pane", "-t", parts[1]]).run();
        }
    }

    // Restore saved layouts
    for window_id in &windows_with_sidebars {
        restore_window_layout(window_id);
    }
}

/// Save a window's layout to a tmux window option.
fn save_window_layout(window_id: &str) {
    if let Ok(layout) = Cmd::new("tmux")
        .args(&["display-message", "-t", window_id, "-p", "#{window_layout}"])
        .run_and_capture_stdout()
    {
        let layout = layout.trim();
        if !layout.is_empty() {
            let _ = Cmd::new("tmux")
                .args(&[
                    "set-option",
                    "-w",
                    "-t",
                    window_id,
                    "@workmux_sidebar_layout",
                    layout,
                ])
                .run();
        }
    }
}

/// Restore a window's layout from the saved tmux window option.
fn restore_window_layout(window_id: &str) {
    if let Ok(layout) = Cmd::new("tmux")
        .args(&[
            "show-option",
            "-wqv",
            "-t",
            window_id,
            "@workmux_sidebar_layout",
        ])
        .run_and_capture_stdout()
    {
        let layout = layout.trim();
        if !layout.is_empty() {
            let _ = Cmd::new("tmux")
                .args(&["select-layout", "-t", window_id, layout])
                .run();
            let _ = Cmd::new("tmux")
                .args(&[
                    "set-option",
                    "-wu",
                    "-t",
                    window_id,
                    "@workmux_sidebar_layout",
                ])
                .run();
        }
    }
}

/// Install tmux hooks so new windows automatically get a sidebar.
fn install_hooks() -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_str().ok_or_else(|| anyhow!("exe path not UTF-8"))?;

    let sync_cmd = format!(
        "run-shell -b '{} _sidebar-sync --window #{{window_id}}'",
        exe_str
    );

    Cmd::new("tmux")
        .args(&["set-hook", "-g", "after-new-window[99]", &sync_cmd])
        .run()?;
    Cmd::new("tmux")
        .args(&["set-hook", "-g", "after-new-session[99]", &sync_cmd])
        .run()?;

    // Snap sidebar panes to responsive width on terminal resize (10% of client width, clamped)
    let resize_script = format!(
        r#"cw=$(tmux display-message -p '#{{client_width}}'); w=$((cw * 10 / 100)); [ "$w" -lt {min} ] && w={min}; [ "$w" -gt {max} ] && w={max}; tmux set-option -g @workmux_sidebar_width "$w"; tmux list-panes -a -F '#{{pane_id}} #{{@workmux_role}}' | while read id role; do [ "$role" = "sidebar" ] && tmux resize-pane -t "$id" -x "$w"; done"#,
        min = MIN_WIDTH,
        max = MAX_WIDTH,
    );
    let resize_cmd = format!("run-shell -b \"{}\"", resize_script);
    Cmd::new("tmux")
        .args(&["set-hook", "-g", "client-resized[99]", &resize_cmd])
        .run()?;

    Ok(())
}

/// Remove tmux hooks.
fn remove_hooks() {
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "after-new-window[99]"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "after-new-session[99]"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "client-resized[99]"])
        .run();
}

/// Check if the sidebar is the only pane left in its window.
fn is_last_pane_in_window() -> bool {
    Cmd::new("tmux")
        .args(&["list-panes", "-F", "#{pane_id}"])
        .run_and_capture_stdout()
        .map(|s| s.lines().count() <= 1)
        .unwrap_or(false)
}

/// Shut down all sidebars globally (called when any sidebar quits).
/// Kills all other sidebar panes immediately, then defers our own window's
/// layout restore so it fires after our process exits and the pane closes.
fn shutdown_all_sidebars() {
    let our_pane = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{pane_id}"])
        .run_and_capture_stdout()
        .unwrap_or_default()
        .trim()
        .to_string();
    let our_window = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{window_id}"])
        .run_and_capture_stdout()
        .unwrap_or_default()
        .trim()
        .to_string();

    let output = Cmd::new("tmux")
        .args(&[
            "list-panes",
            "-a",
            "-F",
            "#{window_id} #{pane_id} #{@workmux_role}",
        ])
        .run_and_capture_stdout()
        .unwrap_or_default();

    let mut other_windows = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() == 3 && parts[2].trim() == SIDEBAR_ROLE_VALUE {
            let pane_id = parts[1].trim();
            if pane_id != our_pane {
                other_windows.push(parts[0].to_string());
                let _ = Cmd::new("tmux").args(&["kill-pane", "-t", pane_id]).run();
            }
        }
    }

    // Restore layouts for other windows
    for window_id in &other_windows {
        restore_window_layout(window_id);
    }

    // Remove hooks and global options
    remove_hooks();
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-gu", "@workmux_sidebar_enabled"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-gu", "@workmux_sidebar_width"])
        .run();

    // Defer our own window's layout restore until after our pane closes
    if !our_window.is_empty()
        && let Ok(layout) = Cmd::new("tmux")
            .args(&[
                "show-option",
                "-wqv",
                "-t",
                &our_window,
                "@workmux_sidebar_layout",
            ])
            .run_and_capture_stdout()
    {
        let layout = layout.trim().to_string();
        if !layout.is_empty() {
            let cmd = format!(
                "sleep 0.1 && tmux select-layout -t {win} '{layout}' && tmux set-option -wu -t {win} @workmux_sidebar_layout",
                win = our_window,
                layout = layout,
            );
            let _ = Cmd::new("tmux").args(&["run-shell", "-b", &cmd]).run();
        }
    }
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

            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _)
                | (KeyCode::Esc, _)
                | (KeyCode::Char('c'), crossterm::event::KeyModifiers::CONTROL) => {
                    app.should_quit = true;
                }
                (KeyCode::Char('j'), _) | (KeyCode::Down, _) => {
                    app.next();
                }
                (KeyCode::Char('k'), _) | (KeyCode::Up, _) => {
                    app.previous();
                }
                (KeyCode::Enter, _) => {
                    app.jump_to_selected();
                }
                (KeyCode::Char('G'), _) => {
                    app.select_last();
                }
                (KeyCode::Char('g'), _) => {
                    app.select_first();
                }
                (KeyCode::Char('v'), _) => {
                    app.toggle_layout_mode();
                }
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
            app.tick();
            app.update_active_window();
        }

        // Auto-refresh agent list and sync layout mode
        if last_refresh.elapsed() >= refresh_interval {
            app.refresh();
            app.sync_layout_mode();
            last_refresh = std::time::Instant::now();

            // Quit if we're the last pane in the window
            if is_last_pane_in_window() {
                app.should_quit = true;
            }
        }

        if app.should_quit {
            shutdown_all_sidebars();
            break;
        }
    }

    // _guard handles cleanup on drop (including the normal exit path)
    Ok(())
}
