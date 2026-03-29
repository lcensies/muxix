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
use std::time::{Duration, Instant, SystemTime};

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

/// Tracked mtime state for a worktree's git internals.
#[derive(Default)]
struct GitMtimes {
    /// mtime of .git/index (changes on stage/unstage/commit)
    index: Option<SystemTime>,
    /// mtime of .git/HEAD (changes on commit/checkout/branch switch)
    head: Option<SystemTime>,
    /// mtime of the branch ref file (changes on commit)
    branch_ref: Option<SystemTime>,
}

/// Get the mtime of a file, returning None if the file doesn't exist.
fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

/// Resolve the .git directory for a worktree path.
/// For linked worktrees, .git is a file containing "gitdir: /path/to/real/gitdir".
fn resolve_git_dir(worktree_path: &Path) -> Option<PathBuf> {
    let dot_git = worktree_path.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    if dot_git.is_file() {
        // Linked worktree: read the gitdir pointer
        let content = std::fs::read_to_string(&dot_git).ok()?;
        let gitdir = content.strip_prefix("gitdir: ")?.trim();
        let path = PathBuf::from(gitdir);
        if path.is_absolute() {
            return Some(path);
        }
        // Relative path: resolve relative to worktree
        Some(worktree_path.join(path))
    } else {
        None
    }
}

/// Check if any tracked git internal files have changed since last check.
fn git_mtimes_changed(worktree_path: &Path, git_dir: &Path, prev: &GitMtimes) -> (bool, GitMtimes) {
    let index = file_mtime(&git_dir.join("index"));
    let head = file_mtime(&git_dir.join("HEAD"));

    // Try to find the branch ref file for commit detection
    let branch_ref = std::fs::read_to_string(git_dir.join("HEAD"))
        .ok()
        .and_then(|content| {
            let trimmed = content.trim();
            let ref_path = trimmed.strip_prefix("ref: ")?;
            // Check worktree-specific refs first, then shared git dir
            let worktree_ref = git_dir.join(ref_path);
            if worktree_ref.exists() {
                return file_mtime(&worktree_ref);
            }
            // For linked worktrees, check the common dir
            let common_dir = git_dir.join("commondir");
            if let Ok(common) = std::fs::read_to_string(&common_dir) {
                let common_path = git_dir.join(common.trim()).join(ref_path);
                return file_mtime(&common_path);
            }
            // Direct path
            file_mtime(&worktree_path.join(".git").join(ref_path))
        });

    let current = GitMtimes {
        index,
        head,
        branch_ref,
    };

    let changed = current.index != prev.index
        || current.head != prev.head
        || current.branch_ref != prev.branch_ref;

    (changed, current)
}

/// Spawn a background thread that watches for git changes and updates the cache.
///
/// Uses mtime-based change detection on .git internals (index, HEAD, branch ref)
/// to avoid running expensive git subprocesses when nothing changed. When mtimes
/// change, immediately refreshes and sets dirty_flag for instant broadcast.
/// Falls back to a periodic full sweep every 30s for uncommitted file edits.
fn spawn_git_worker(
    term: Arc<AtomicBool>,
    dirty_flag: Arc<AtomicBool>,
) -> (GitCache, std::sync::mpsc::Sender<Vec<PathBuf>>) {
    let cache: GitCache = Arc::new(Mutex::new(HashMap::new()));
    let cache_clone = cache.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<PathBuf>>();

    thread::spawn(move || {
        let mut active_paths: Vec<PathBuf> = Vec::new();
        let mut mtimes: HashMap<PathBuf, GitMtimes> = HashMap::new();
        let mut git_dirs: HashMap<PathBuf, PathBuf> = HashMap::new();
        let mut last_full_sweep = Instant::now();
        let mut last_dirty_probe = Instant::now();
        let full_sweep_interval = Duration::from_secs(30);
        // Working tree dirty probe runs every 3s (subprocess is expensive).
        // Mtime checks still run every 1s (free stat() calls).
        let dirty_probe_interval = Duration::from_secs(3);

        while !term.load(Ordering::Relaxed) {
            // Drain channel to get the latest set of active paths
            while let Ok(paths) = rx.try_recv() {
                active_paths = paths;
            }

            // Deduplicate paths (multiple panes can share a worktree)
            let mut unique_paths: Vec<PathBuf> = active_paths.clone();
            unique_paths.sort();
            unique_paths.dedup();

            // Resolve git dirs for any new paths
            for path in &unique_paths {
                git_dirs
                    .entry(path.clone())
                    .or_insert_with(|| resolve_git_dir(path).unwrap_or_else(|| path.join(".git")));
            }

            let force_full = last_full_sweep.elapsed() >= full_sweep_interval;
            if force_full {
                last_full_sweep = Instant::now();
            }

            // Only run the working tree dirty probe every 3s to limit subprocess spawns
            let run_dirty_probe = last_dirty_probe.elapsed() >= dirty_probe_interval;
            if run_dirty_probe {
                last_dirty_probe = Instant::now();
            }

            let mut any_changed = false;

            for path in &unique_paths {
                let git_dir = match git_dirs.get(path) {
                    Some(d) => d,
                    None => continue,
                };

                let prev_mtimes = mtimes.remove(path).unwrap_or_default();
                let (mtimes_changed, new_mtimes) = git_mtimes_changed(path, git_dir, &prev_mtimes);
                mtimes.insert(path.clone(), new_mtimes);

                // First time seeing this path (no cached status yet)
                let is_new = cache_clone
                    .lock()
                    .ok()
                    .map(|c| !c.contains_key(path))
                    .unwrap_or(true);

                // Working tree dirty probe using `git diff-files --quiet`.
                // Cheaper than `git diff --quiet HEAD`: only compares index stat
                // cache vs working tree metadata, no git object reads needed.
                // Staging is already caught by .git/index mtime checks.
                // Only run every 3s to limit subprocess spawns.
                let worktree_changed = run_dirty_probe
                    && !mtimes_changed
                    && !is_new
                    && !force_full
                    && Cmd::new("git")
                        .workdir(path)
                        .args(&["diff-files", "--quiet"])
                        .run_as_check()
                        .map(|clean| !clean)
                        .unwrap_or(false);

                if !mtimes_changed && !is_new && !force_full && !worktree_changed {
                    continue;
                }

                let new_status = crate::git::get_git_status(path, None);

                // Check if the status actually changed before flagging dirty
                let status_changed = cache_clone
                    .lock()
                    .ok()
                    .map(|c| c.get(path) != Some(&new_status))
                    .unwrap_or(true);

                if let Ok(mut c) = cache_clone.lock() {
                    c.insert(path.clone(), new_status);
                }

                if status_changed {
                    any_changed = true;
                }
            }

            // Prune paths no longer in the active set
            if let Ok(mut c) = cache_clone.lock() {
                let before = c.len();
                c.retain(|p, _| unique_paths.contains(p));
                if c.len() != before {
                    any_changed = true;
                }
            }
            mtimes.retain(|p, _| unique_paths.contains(p));
            git_dirs.retain(|p, _| unique_paths.contains(p));

            // Signal daemon for immediate broadcast when cache changed
            if any_changed {
                dirty_flag.store(true, Ordering::Relaxed);
            }

            // Poll every 1s for mtime checks (free stat() calls).
            // Working tree dirty probe only runs every 3s (see above).
            thread::sleep(Duration::from_secs(1));
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

    // Background git status worker (shares dirty_flag for immediate broadcast on changes)
    let (git_cache, git_path_tx) = spawn_git_worker(term.clone(), dirty_flag.clone());

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
