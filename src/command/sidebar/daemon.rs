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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::cmd::Cmd;
use crate::config::Config;
use crate::git::GitStatus;
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
    pane_window_ids: HashMap<String, String>,
    active_pane_ids: HashSet<String>,
    window_pane_counts: HashMap<String, usize>,
}

/// Query all sidebar-relevant tmux state in a single command.
fn query_tmux_state() -> TmuxState {
    let format = "#{pane_id}\t#{session_name}\t#{window_id}\t#{@workmux_status}\t#{window_active}\t#{session_attached}\t#{pane_active}";
    let output = Cmd::new("tmux")
        .args(&["list-panes", "-a", "-F", format])
        .run_and_capture_stdout()
        .unwrap_or_default();

    let mut window_statuses = HashMap::new();
    let mut active_windows = HashSet::new();
    let mut pane_window_ids = HashMap::new();
    let mut active_pane_ids = HashSet::new();
    let mut window_pane_counts: HashMap<String, usize> = HashMap::new();

    for line in output.lines() {
        let mut parts = line.split('\t');
        let (
            Some(pane_id),
            Some(session),
            Some(window_id),
            Some(status),
            Some(win_active),
            Some(sess_attached),
            Some(pane_active),
        ) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        )
        else {
            continue;
        };
        let win_active = win_active == "1";
        let sess_attached = sess_attached == "1";
        let pane_active = pane_active == "1";

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
        if pane_active {
            active_pane_ids.insert(pane_id.to_string());
        }
    }

    TmuxState {
        window_statuses,
        active_windows,
        pane_window_ids,
        active_pane_ids,
        window_pane_counts,
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

/// Read the sidebar layout mode from tmux global, falling back to settings.json, then config.
fn read_sidebar_layout_mode(config: &Config) -> Option<SidebarLayoutMode> {
    // Check tmux global first (set by toggle_layout_mode during this session)
    if let Ok(output) = Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@workmux_sidebar_layout"])
        .run_and_capture_stdout()
    {
        match output.trim() {
            "tiles" => return Some(SidebarLayoutMode::Tiles),
            "compact" => return Some(SidebarLayoutMode::Compact),
            _ => {}
        }
    }

    // Fall back to persisted setting (user toggled layout in a previous tmux session)
    if let Ok(store) = StateStore::new()
        && let Ok(settings) = store.load_settings()
    {
        match settings.sidebar_layout.as_deref() {
            Some("tiles") => return Some(SidebarLayoutMode::Tiles),
            Some("compact") => return Some(SidebarLayoutMode::Compact),
            _ => {}
        }
    }

    // Fall back to config file
    match config.sidebar.layout.as_deref() {
        Some("tiles") => return Some(SidebarLayoutMode::Tiles),
        Some("compact") => return Some(SidebarLayoutMode::Compact),
        _ => {}
    }

    None
}

/// Shared git status cache, updated by a background worker thread.
type GitCache = Arc<Mutex<HashMap<PathBuf, GitStatus>>>;

/// Spawn a background thread that periodically fetches git status for active agent paths.
/// Returns the shared cache and a channel sender to update the set of active paths.
fn spawn_git_worker(term: Arc<AtomicBool>) -> (GitCache, std::sync::mpsc::Sender<Vec<PathBuf>>) {
    let cache: GitCache = Arc::new(Mutex::new(HashMap::new()));
    let cache_clone = cache.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<PathBuf>>();

    thread::spawn(move || {
        let mut active_paths: Vec<PathBuf> = Vec::new();
        let git_ttl_secs = 5;

        while !term.load(Ordering::Relaxed) {
            // Drain channel to get the latest set of active paths
            while let Ok(paths) = rx.try_recv() {
                active_paths = paths;
            }

            // Deduplicate paths (multiple panes can share a worktree)
            let mut unique_paths: Vec<PathBuf> = active_paths.clone();
            unique_paths.sort();
            unique_paths.dedup();

            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();

            for path in &unique_paths {
                // Skip if cached and still fresh
                let is_stale = cache_clone
                    .lock()
                    .ok()
                    .and_then(|c| c.get(path).and_then(|s| s.cached_at))
                    .map(|ts| now.saturating_sub(ts) >= git_ttl_secs)
                    .unwrap_or(true);

                if !is_stale {
                    continue;
                }

                let status = crate::git::get_git_status(path, None);
                if let Ok(mut c) = cache_clone.lock() {
                    c.insert(path.clone(), status);
                }
            }

            // Prune paths no longer in the active set
            if let Ok(mut c) = cache_clone.lock() {
                c.retain(|p, _| unique_paths.contains(p));
            }

            thread::sleep(Duration::from_secs(2));
        }
    });

    (cache, tx)
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

    // Background git status worker
    let (git_cache, git_path_tx) = spawn_git_worker(term.clone());

    // Store PID so toggle-off can kill us and hooks can signal us
    Cmd::new("tmux")
        .args(&[
            "set-option",
            "-g",
            "@workmux_sidebar_daemon_pid",
            &std::process::id().to_string(),
        ])
        .run()?;

    let mut last_refresh = Instant::now();
    let mut last_client_seen = Instant::now();
    let mut dirty_pending = false;
    let mut last_agent_list = String::new();
    let refresh_interval = Duration::from_secs(2);
    let debounce_interval = Duration::from_millis(50);

    while !term.load(Ordering::Relaxed) {
        // Coalesce dirty signals: SIGUSR1 sets the flag, we service it once
        // per debounce interval to prevent signal floods from causing CPU storms
        if dirty_flag.swap(false, Ordering::Relaxed) {
            dirty_pending = true;
        }

        let time_since_refresh = last_refresh.elapsed();
        let debounce_cleared = dirty_pending && time_since_refresh >= debounce_interval;
        let timer_expired = time_since_refresh >= refresh_interval;

        if debounce_cleared || timer_expired {
            dirty_pending = false;
            last_refresh = Instant::now();

            if let Some(snapshot) = try_build_snapshot(&mux, &status_icons, &config, &git_cache) {
                // Update git worker with current agent paths
                let paths: Vec<PathBuf> = snapshot.agents.iter().map(|a| a.path.clone()).collect();
                let _ = git_path_tx.send(paths);

                server.broadcast(&snapshot);

                let agent_list: String = snapshot
                    .agents
                    .iter()
                    .map(|a| a.pane_id.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");

                if agent_list != last_agent_list {
                    if !agent_list.is_empty() {
                        let _ = Cmd::new("tmux")
                            .args(&["set-option", "-g", "@workmux_sidebar_agents", &agent_list])
                            .run();
                    } else {
                        let _ = Cmd::new("tmux")
                            .args(&["set-option", "-gu", "@workmux_sidebar_agents"])
                            .run();
                    }
                    last_agent_list = agent_list;
                }
            }
        }

        // Track client activity for auto-exit
        if server.client_count() > 0 {
            last_client_seen = Instant::now();
        } else if last_client_seen.elapsed() > Duration::from_secs(10) {
            break;
        }

        // Always sleep to prevent CPU spinning (never skip on dirty)
        thread::sleep(Duration::from_millis(10));
    }

    // Cleanup
    let _ = std::fs::remove_file(&sock_path);
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-gu", "@workmux_sidebar_daemon_pid"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-gu", "@workmux_sidebar_agents"])
        .run();
    Ok(())
}

/// Try to build a snapshot. Returns None on transient failures.
fn try_build_snapshot(
    mux: &Arc<dyn Multiplexer>,
    status_icons: &crate::config::StatusIcons,
    config: &Config,
    git_cache: &GitCache,
) -> Option<super::snapshot::SidebarSnapshot> {
    let tmux_state = query_tmux_state();
    let agents = StateStore::new()
        .and_then(|store| store.load_reconciled_agents(mux.as_ref()))
        .ok()?;
    let layout_mode = read_sidebar_layout_mode(config).unwrap_or_default();

    let git_statuses = git_cache.lock().ok().map(|c| c.clone()).unwrap_or_default();

    Some(build_snapshot(
        agents,
        &tmux_state.window_statuses,
        &tmux_state.pane_window_ids,
        tmux_state.active_windows,
        tmux_state.active_pane_ids,
        tmux_state.window_pane_counts,
        layout_mode,
        status_icons,
        git_statuses,
    ))
}
