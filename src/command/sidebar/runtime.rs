//! TUI event loop for the sidebar client.

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::backend::CrosstermBackend;
use std::io;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::multiplexer::{create_backend, detect_backend};

use super::app::SidebarApp;
use super::client;
use super::daemon_ctrl::{ensure_daemon_running, signal_daemon};
use super::panes::shutdown_all_sidebars;
use super::ui::render_sidebar;

/// Drop guard that restores terminal state on panic or early return.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

enum AppEvent {
    /// A new snapshot is available in the SnapshotHandle.
    SnapshotReady,
    /// A terminal input event (key press, resize, etc.).
    Input(Event),
}

/// Spawn a thread that reads terminal events and forwards them.
/// Must be called AFTER terminal raw mode is enabled.
fn spawn_input_thread(tx: mpsc::Sender<AppEvent>) {
    thread::spawn(move || {
        // event::read() blocks until input is available - zero CPU
        while let Ok(ev) = event::read() {
            if tx.send(AppEvent::Input(ev)).is_err() {
                break;
            }
        }
    });
}

/// Run the sidebar TUI (called by the hidden `_sidebar-run` command).
pub fn run_sidebar() -> Result<()> {
    let mux = create_backend(detect_backend());

    if !mux.is_running().unwrap_or(false) {
        return Ok(());
    }

    // Ensure daemon is running (may have auto-exited or crashed)
    let sock_path = ensure_daemon_running()?;

    // Setup terminal FIRST (raw mode required before spawning input thread)
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _guard = TerminalGuard;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    // Channel for all events
    let (tx, rx) = mpsc::channel();

    // Snapshot receiver: overwrites latest, sends SnapshotReady wake via
    // a thin forwarding thread that converts () -> AppEvent::SnapshotReady
    let snapshot_handle = {
        let (wake_tx, wake_rx) = mpsc::sync_channel::<()>(1);
        let event_tx = tx.clone();
        thread::spawn(move || {
            for () in wake_rx {
                if event_tx.send(AppEvent::SnapshotReady).is_err() {
                    break;
                }
            }
        });
        client::connect(&sock_path, wake_tx)
    };

    // Input reader thread (terminal is already in raw mode)
    spawn_input_thread(tx);

    // Signal daemon to push an immediate snapshot for the newly connected client
    signal_daemon();

    let mut app = SidebarApp::new_client(mux)?;
    let mut needs_render = true;
    let startup = std::time::Instant::now();
    let startup_grace = Duration::from_secs(3);

    loop {
        // Render before blocking (redraws only when state changed)
        if needs_render {
            terminal.draw(|f| render_sidebar(f, &mut app))?;
            needs_render = false;
        }

        // Adaptive timeout: 250ms when active (for spinner), block when hidden
        let timeout = if app.host_window_active() {
            Duration::from_millis(250)
        } else {
            // Block until a snapshot or input wakes us. Use a large timeout
            // since recv() without timeout would prevent clean shutdown if
            // all senders drop.
            Duration::from_secs(3600)
        };

        let first_event = match rx.recv_timeout(timeout) {
            Ok(ev) => Some(ev),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Spinner tick (only fires when active, guaranteed by timeout choice)
                if app.host_window_active() {
                    app.tick();
                    needs_render = true;
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        // Process first event
        if let Some(ev) = first_event {
            process_event(
                ev,
                &mut app,
                &snapshot_handle,
                &startup,
                startup_grace,
                &mut needs_render,
            );
        }

        // Drain all pending events to coalesce (avoids multiple redraws)
        while let Ok(ev) = rx.try_recv() {
            process_event(
                ev,
                &mut app,
                &snapshot_handle,
                &startup,
                startup_grace,
                &mut needs_render,
            );
        }

        if app.should_quit {
            shutdown_all_sidebars();
            break;
        }
    }

    // _guard handles cleanup on drop (including the normal exit path)
    Ok(())
}

fn process_event(
    event: AppEvent,
    app: &mut SidebarApp,
    snapshot_handle: &client::SnapshotHandle,
    startup: &std::time::Instant,
    startup_grace: Duration,
    needs_render: &mut bool,
) {
    match event {
        AppEvent::SnapshotReady => {
            if let Some(snapshot) = snapshot_handle.take() {
                // Check last-pane using snapshot data (with startup grace period)
                if startup.elapsed() > startup_grace
                    && let Some(wid) = app.host_window_id()
                    && snapshot.window_pane_counts.get(wid).copied().unwrap_or(2) <= 1
                {
                    app.should_quit = true;
                }
                app.apply_snapshot(snapshot);
                *needs_render = true;
            }
        }
        AppEvent::Input(Event::Key(key)) if key.kind == KeyEventKind::Press => {
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _)
                | (KeyCode::Esc, _)
                | (KeyCode::Char('c'), crossterm::event::KeyModifiers::CONTROL) => {
                    app.should_quit = true;
                }
                (KeyCode::Char('j'), _) | (KeyCode::Down, _) => app.next(),
                (KeyCode::Char('k'), _) | (KeyCode::Up, _) => app.previous(),
                (KeyCode::Enter, _) => app.jump_to_selected(),
                (KeyCode::Char('G'), _) => app.select_last(),
                (KeyCode::Char('g'), _) => app.select_first(),
                (KeyCode::Char('v'), _) => app.toggle_layout_mode(),
                _ => {}
            }
            *needs_render = true;
        }
        AppEvent::Input(Event::Resize(_, _)) => {
            *needs_render = true;
        }
        AppEvent::Input(_) => {}
    }
}
