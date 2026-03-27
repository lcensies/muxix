//! Sidebar daemon: single process that polls tmux and pushes snapshots to clients.

use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::cmd::Cmd;
use crate::config::Config;
use crate::multiplexer::{Multiplexer, create_backend, detect_backend};
use crate::state::StateStore;

use super::app::SidebarLayoutMode;
use super::snapshot::build_snapshot;

/// Compute socket path from instance_id.
pub fn socket_path(instance_id: &str) -> PathBuf {
    let safe_id = instance_id.replace(['/', '\\'], "-");
    std::env::temp_dir().join(format!("workmux-sidebar-{}.sock", safe_id))
}

/// Result of a batched tmux query.
struct TmuxState {
    window_statuses: HashMap<String, Option<String>>,
    active_windows: HashSet<(String, String)>,
    window_pane_counts: HashMap<String, usize>,
    pane_window_ids: HashMap<String, String>,
}

/// Query all sidebar-relevant tmux state in a single command.
fn query_tmux_state() -> TmuxState {
    let format = "#{pane_id}\t#{session_name}\t#{window_name}\t#{window_id}\t#{@workmux_status}\t#{window_active}\t#{session_attached}";
    let output = Cmd::new("tmux")
        .args(&["list-panes", "-a", "-F", format])
        .run_and_capture_stdout()
        .unwrap_or_default();

    let mut window_statuses = HashMap::new();
    let mut active_windows = HashSet::new();
    let mut window_pane_counts: HashMap<String, usize> = HashMap::new();
    let mut pane_window_ids = HashMap::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 7 {
            continue;
        }

        let pane_id = parts[0];
        let session = parts[1];
        let _window_name = parts[2];
        let window_id = parts[3];
        let status = parts[4];
        let win_active = parts[5] == "1";
        let sess_attached = parts[6] == "1";

        let status_val = if status.is_empty() {
            None
        } else {
            Some(status.to_string())
        };
        window_statuses.insert(pane_id.to_string(), status_val);
        pane_window_ids.insert(pane_id.to_string(), window_id.to_string());

        *window_pane_counts.entry(window_id.to_string()).or_default() += 1;

        if win_active && sess_attached {
            active_windows.insert((session.to_string(), window_id.to_string()));
        }
    }

    TmuxState {
        window_statuses,
        active_windows,
        window_pane_counts,
        pane_window_ids,
    }
}

/// Unix socket server for broadcasting snapshots to clients.
struct SocketServer {
    clients: Arc<Mutex<Vec<UnixStream>>>,
}

impl SocketServer {
    fn bind(path: &Path) -> std::io::Result<Self> {
        let listener = UnixListener::bind(path)?;
        // Restrict socket to owner only (prevent other local users from reading snapshots)
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let clients: Arc<Mutex<Vec<UnixStream>>> = Arc::new(Mutex::new(Vec::new()));
        let clients_clone = clients.clone();

        thread::spawn(move || {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        // 1ms write timeout: local Unix sockets shouldn't block
                        let _ = stream.set_write_timeout(Some(Duration::from_millis(1)));
                        clients_clone.lock().unwrap().push(stream);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self { clients })
    }

    fn broadcast(&self, snapshot: &super::snapshot::SidebarSnapshot) {
        let data = serde_json::to_vec(snapshot).unwrap_or_default();
        let len = (data.len() as u32).to_be_bytes();

        // Take clients out of mutex to avoid holding lock during writes
        let mut clients = std::mem::take(&mut *self.clients.lock().unwrap());
        clients
            .retain_mut(|stream| stream.write_all(&len).is_ok() && stream.write_all(&data).is_ok());
        // Merge surviving clients back (append to preserve any new connections accepted during writes)
        self.clients.lock().unwrap().append(&mut clients);
    }

    fn client_count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }
}

/// Read the sidebar layout mode from the tmux global option.
fn read_sidebar_layout_mode() -> Option<SidebarLayoutMode> {
    let output = Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@workmux_sidebar_layout"])
        .run_and_capture_stdout()
        .ok()?;
    match output.trim() {
        "tiles" => Some(SidebarLayoutMode::Tiles),
        "compact" => Some(SidebarLayoutMode::Compact),
        _ => None,
    }
}

/// Run the sidebar daemon (headless, no TUI).
pub fn run() -> Result<()> {
    let mux = create_backend(detect_backend());
    let instance_id = mux.instance_id();
    let config = Config::load(None)?;
    let status_icons = config.status_icons.clone();

    let sock_path = socket_path(&instance_id);
    let _ = std::fs::remove_file(&sock_path); // Clean stale
    let server = SocketServer::bind(&sock_path)?;

    // Signal handlers for clean shutdown and dirty notification
    let term = Arc::new(AtomicBool::new(false));
    let dirty_flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, term.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGUSR1, dirty_flag.clone())?;

    // Store PID so toggle-off can kill us and hooks can signal us
    Cmd::new("tmux")
        .args(&[
            "set-option",
            "-g",
            "@workmux_sidebar_daemon_pid",
            &std::process::id().to_string(),
        ])
        .run()?;

    let mut version = 0u64;
    let mut last_refresh = Instant::now();
    let mut last_client_seen = Instant::now();
    let refresh_interval = Duration::from_secs(2);

    while !term.load(Ordering::Relaxed) {
        let dirty = dirty_flag.swap(false, Ordering::Relaxed);
        let timer_expired = last_refresh.elapsed() >= refresh_interval;

        if dirty || timer_expired {
            last_refresh = Instant::now();

            if let Some(snapshot) = try_build_snapshot(&mux, &status_icons, &mut version) {
                server.broadcast(&snapshot);
            }
        }

        // Track client activity for auto-exit
        if server.client_count() > 0 {
            last_client_seen = Instant::now();
        } else if last_client_seen.elapsed() > Duration::from_secs(10) {
            break;
        }

        // Skip sleep after processing a dirty signal to minimize latency;
        // Rust's thread::sleep retries on EINTR so SIGUSR1 can't interrupt it
        if !dirty {
            thread::sleep(Duration::from_millis(50));
        }
    }

    // Cleanup
    let _ = std::fs::remove_file(&sock_path);
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-gu", "@workmux_sidebar_daemon_pid"])
        .run();
    Ok(())
}

/// Try to build a snapshot. Returns None on transient failures.
fn try_build_snapshot(
    mux: &Arc<dyn Multiplexer>,
    status_icons: &crate::config::StatusIcons,
    version: &mut u64,
) -> Option<super::snapshot::SidebarSnapshot> {
    let tmux_state = query_tmux_state();
    let agents = StateStore::new()
        .and_then(|store| store.load_reconciled_agents(mux.as_ref()))
        .ok()?;
    let layout_mode = read_sidebar_layout_mode().unwrap_or_default();

    *version += 1;
    Some(build_snapshot(
        agents,
        &tmux_state.window_statuses,
        &tmux_state.pane_window_ids,
        tmux_state.active_windows,
        tmux_state.window_pane_counts,
        layout_mode,
        status_icons,
        *version,
    ))
}
