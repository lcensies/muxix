//! TUI event loop for the sidebar client.

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::backend::CrosstermBackend;
use std::io;
use std::time::Duration;

use crate::multiplexer::{create_backend, detect_backend};

use super::app::SidebarApp;
use super::client;
use super::daemon_ctrl::{ensure_daemon_running, signal_daemon};
use super::panes::{is_last_pane_in_window, shutdown_all_sidebars};
use super::ui::render_sidebar;

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

    // Ensure daemon is running (may have auto-exited or crashed)
    let sock_path = ensure_daemon_running()?;

    // Connect to daemon (retries in background thread)
    let receiver = client::SnapshotReceiver::connect(&sock_path);

    // Signal daemon to push an immediate snapshot for the newly connected client
    signal_daemon();

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    // Drop guard ensures terminal is restored even on panic/error
    let _guard = TerminalGuard;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    let mut app = SidebarApp::new_client(mux)?;

    // Main loop: 10ms tick for responsive snapshot pickup, spinner every ~250ms
    let tick_rate = Duration::from_millis(10);
    let mut last_tick = std::time::Instant::now();
    let mut spin_counter = 0u32;
    let last_pane_check_interval = Duration::from_secs(2);
    let mut last_pane_check = std::time::Instant::now();

    let mut needs_render = true; // Draw on first iteration

    loop {
        // Apply latest snapshot
        if let Some(snapshot) = receiver.take() {
            app.apply_snapshot(snapshot);
            needs_render = true;
        }

        // Only redraw when state changed (snapshot, key press, spinner tick, resize)
        if needs_render {
            terminal.draw(|f| render_sidebar(f, &mut app))?;
            needs_render = false;
        }

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());

        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
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
                    needs_render = true;
                }
                Event::Resize(_, _) => {
                    needs_render = true;
                }
                _ => {} // Non-key events: no continue, bookkeeping always runs
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
            spin_counter += 1;
            // Tick spinner every ~250ms (every 25th tick at 10ms)
            if spin_counter.is_multiple_of(25) {
                app.tick();
                needs_render = true;
            }
        }

        // Check if last pane periodically
        if last_pane_check.elapsed() >= last_pane_check_interval {
            last_pane_check = std::time::Instant::now();
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
