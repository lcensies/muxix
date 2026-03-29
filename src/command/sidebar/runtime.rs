//! TUI event loop for the sidebar client.

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::backend::CrosstermBackend;
use std::io;
use std::io::Write as IoWrite;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn dbg_log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/workmux-sidebar-debug.log")
    {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let pid = std::process::id();
        // Format entire line first, then single write_all for atomic O_APPEND
        let line = format!("[{:.3}] P{}: {}\n", now.as_secs_f64(), pid, msg);
        let _ = f.write_all(line.as_bytes());
    }
}

use crate::multiplexer::{create_backend, detect_backend};

use super::app::SidebarApp;
use super::client;
use super::daemon_ctrl::ensure_daemon_running;
use super::panes::shutdown_all_sidebars;
use super::ui::render_sidebar;

/// Drop guard that restores terminal state on panic or early return.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
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

    // Create app BEFORE entering raw mode: terminal_light::luma() queries
    // the terminal via stdin, which would race with the input reader thread.
    let mut app = SidebarApp::new_client(mux)?;
    dbg_log(&format!(
        "app created: agents={} host_active={} host_wid={:?}",
        app.agents.len(),
        app.host_window_active(),
        app.host_window_id(),
    ));

    // Ensure daemon is running (may have auto-exited or crashed)
    let sock_path = ensure_daemon_running()?;

    // Setup terminal (raw mode required before spawning input thread)
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
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

    dbg_log("input thread started, waiting for snapshot");
    let mut needs_render = true;
    let startup = std::time::Instant::now();
    let startup_grace = Duration::from_secs(3);

    loop {
        // Render before blocking (redraws only when state changed)
        if needs_render {
            dbg_log(&format!(
                "RENDER: agents={} selected={:?} host_active={} term={}x{}",
                app.agents.len(),
                app.list_state.selected(),
                app.host_window_active(),
                terminal.size().map(|s| s.width).unwrap_or(0),
                terminal.size().map(|s| s.height).unwrap_or(0),
            ));
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

        let wait_start = std::time::Instant::now();
        let first_event = match rx.recv_timeout(timeout) {
            Ok(ev) => Some(ev),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Spinner tick (only fires when active, guaranteed by timeout choice)
                if app.host_window_active() {
                    app.tick();
                    needs_render = true;
                }
                // Log if we've been waiting a long time with no agents
                if app.agents.is_empty() && startup.elapsed() > Duration::from_secs(1) {
                    dbg_log(&format!(
                        "TIMEOUT: still empty after {:.0}ms, host_active={}, timeout={:?}",
                        startup.elapsed().as_secs_f64() * 1000.0,
                        app.host_window_active(),
                        timeout,
                    ));
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        // Log how long we waited (only when agents are empty to avoid spam)
        if app.agents.is_empty() {
            dbg_log(&format!(
                "EVENT: waited {:.0}ms, agents=0, since_start={:.0}ms",
                wait_start.elapsed().as_secs_f64() * 1000.0,
                startup.elapsed().as_secs_f64() * 1000.0,
            ));
        }

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
                dbg_log(&format!(
                    "SNAPSHOT: agents={} active_windows={:?} host_active_before={} since_start={:.0}ms",
                    snapshot.agents.len(),
                    snapshot.active_windows,
                    app.host_window_active(),
                    startup.elapsed().as_secs_f64() * 1000.0,
                ));
                // Check last-pane using snapshot data (with startup grace period)
                if startup.elapsed() > startup_grace
                    && let Some(wid) = app.host_window_id()
                    && snapshot.window_pane_counts.get(wid).copied().unwrap_or(2) <= 1
                {
                    app.should_quit = true;
                }
                app.apply_snapshot(snapshot);
                dbg_log(&format!(
                    "APPLIED: agents={} host_active={} host_idx={:?}",
                    app.agents.len(),
                    app.host_window_active(),
                    app.host_agent_idx,
                ));
                *needs_render = true;
            } else {
                dbg_log("SNAPSHOT: take() returned None");
            }
        }
        AppEvent::Input(Event::Key(key)) if key.kind == KeyEventKind::Press => {
            dbg_log(&format!("KEY: {:?}", key.code));
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
        AppEvent::Input(Event::Mouse(mouse)) => {
            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(idx) = app.hit_test(mouse.column, mouse.row) {
                        app.select_index(idx);
                        app.jump_to_selected();
                    }
                }
                MouseEventKind::ScrollUp => {
                    app.scroll_up();
                }
                MouseEventKind::ScrollDown => {
                    app.scroll_down();
                }
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
