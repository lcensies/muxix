//! Sidebar daemon: single process that polls tmux and pushes snapshots to clients.

use anyhow::Result;
use notify::{RecursiveMode, Watcher};
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
    let format = "#{pane_id}\t#{session_name}\t#{window_id}\t#{@workmux_pane_status}\t#{window_active}\t#{session_attached}\t#{pane_active}";
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
    fn bind(path: &Path, dirty_flag: Arc<AtomicBool>) -> std::io::Result<Self> {
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
                        // Trigger an immediate broadcast so the new client gets
                        // the current snapshot without waiting for the next timer.
                        dirty_flag.store(true, Ordering::Relaxed);
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

/// Resolve the common git directory for linked worktrees.
/// Returns None for normal (non-linked) worktrees.
fn resolve_common_git_dir(gitdir: &Path) -> Option<PathBuf> {
    let content = std::fs::read_to_string(gitdir.join("commondir")).ok()?;
    let rel = content.trim();
    let path = if Path::new(rel).is_absolute() {
        PathBuf::from(rel)
    } else {
        gitdir.join(rel)
    };
    path.canonicalize().ok().or(Some(path))
}

/// Compare two GitStatus values ignoring the cached_at timestamp.
fn git_status_semantically_equal(a: &GitStatus, b: &GitStatus) -> bool {
    a.ahead == b.ahead
        && a.behind == b.behind
        && a.has_conflict == b.has_conflict
        && a.is_dirty == b.is_dirty
        && a.lines_added == b.lines_added
        && a.lines_removed == b.lines_removed
        && a.uncommitted_added == b.uncommitted_added
        && a.uncommitted_removed == b.uncommitted_removed
        && a.base_branch == b.base_branch
        && a.branch == b.branch
        && a.has_upstream == b.has_upstream
}

/// Find which worktrees are affected by a filesystem event at the given path.
fn find_worktrees_for_path(
    event_path: &Path,
    watch_to_worktrees: &HashMap<PathBuf, HashSet<PathBuf>>,
) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for (watched_dir, worktrees) in watch_to_worktrees {
        if event_path.starts_with(watched_dir) {
            result.extend(worktrees.iter().cloned());
        }
    }
    result
}

/// Register a watch path and associate it with a worktree.
/// If the path is already watched by another worktree, just adds the mapping.
/// Only records the mapping after the OS watch succeeds (or was already active).
fn add_watch(
    watcher: &mut notify::RecommendedWatcher,
    path: &Path,
    mode: RecursiveMode,
    worktree: &Path,
    watch_to_worktrees: &mut HashMap<PathBuf, HashSet<PathBuf>>,
    watched_for_worktree: &mut Vec<PathBuf>,
) {
    let already_watching = watch_to_worktrees.get(path).is_some_and(|s| !s.is_empty());

    if !already_watching && let Err(e) = watcher.watch(path, mode) {
        tracing::warn!("failed to watch {}: {}", path.display(), e);
        return;
    }

    watch_to_worktrees
        .entry(path.to_path_buf())
        .or_default()
        .insert(worktree.to_path_buf());
    watched_for_worktree.push(path.to_path_buf());
}

/// Remove watch association for a worktree. Unwatches the path if no other worktree needs it.
fn remove_worktree_watch(
    watcher: &mut notify::RecommendedWatcher,
    watch_path: &Path,
    worktree: &Path,
    watch_to_worktrees: &mut HashMap<PathBuf, HashSet<PathBuf>>,
) {
    if let Some(worktrees) = watch_to_worktrees.get_mut(watch_path) {
        worktrees.remove(worktree);
        if worktrees.is_empty() {
            watch_to_worktrees.remove(watch_path);
            let _ = watcher.unwatch(watch_path);
        }
    }
}

/// Set up filesystem watches for a worktree.
fn setup_worktree_watches(
    watcher: &mut notify::RecommendedWatcher,
    worktree: &Path,
    watch_to_worktrees: &mut HashMap<PathBuf, HashSet<PathBuf>>,
) -> Vec<PathBuf> {
    let mut watched = Vec::new();
    let dot_git = worktree.join(".git");
    let is_linked = dot_git.is_file();

    if is_linked {
        // Linked worktree: gitdir is outside the worktree root
        if let Some(git_dir) = resolve_git_dir(worktree) {
            // Watch per-worktree gitdir (HEAD, index)
            add_watch(
                watcher,
                &git_dir,
                RecursiveMode::Recursive,
                worktree,
                watch_to_worktrees,
                &mut watched,
            );

            // Watch common dir's refs/ for shared branch updates
            if let Some(common_dir) = resolve_common_git_dir(&git_dir) {
                let refs_dir = common_dir.join("refs");
                if refs_dir.is_dir() {
                    add_watch(
                        watcher,
                        &refs_dir,
                        RecursiveMode::Recursive,
                        worktree,
                        watch_to_worktrees,
                        &mut watched,
                    );
                }
                // Watch common dir non-recursively for packed-refs
                add_watch(
                    watcher,
                    &common_dir,
                    RecursiveMode::NonRecursive,
                    worktree,
                    watch_to_worktrees,
                    &mut watched,
                );
            }
        }
        // Watch worktree root for file edits
        add_watch(
            watcher,
            worktree,
            RecursiveMode::Recursive,
            worktree,
            watch_to_worktrees,
            &mut watched,
        );
    } else {
        // Normal worktree: .git/ is inside, single recursive watch covers everything
        add_watch(
            watcher,
            worktree,
            RecursiveMode::Recursive,
            worktree,
            watch_to_worktrees,
            &mut watched,
        );
    }

    watched
}

/// Calculate the next timeout for the worker's recv_timeout.
/// Returns the shortest wait until either a debounced worktree is ready,
/// the full sweep is due, or a 1s cap for checking the term flag.
fn next_worker_timeout(
    pending: &HashMap<PathBuf, Instant>,
    debounce: Duration,
    last_sweep: Instant,
    sweep_interval: Duration,
) -> Duration {
    let now = Instant::now();
    let sweep_wait = sweep_interval.saturating_sub(last_sweep.elapsed());
    let mut min_wait = sweep_wait;

    for last_event in pending.values() {
        let ready_at = *last_event + debounce;
        if ready_at <= now {
            return Duration::from_millis(1);
        }
        let wait = ready_at - now;
        if wait < min_wait {
            min_wait = wait;
        }
    }

    // Cap at 1s to check term flag periodically
    min_wait.min(Duration::from_secs(1))
}

/// Refresh git status for a worktree path, updating the cache.
/// Returns true if the status actually changed (semantically, ignoring cached_at).
fn refresh_git_status(path: &Path, cache: &GitCache) -> bool {
    let new_status = crate::git::get_git_status(path, None);
    let changed = cache
        .lock()
        .ok()
        .map(|c| {
            c.get(path)
                .is_none_or(|old| !git_status_semantically_equal(old, &new_status))
        })
        .unwrap_or(true);
    if let Ok(mut c) = cache.lock() {
        c.insert(path.to_path_buf(), new_status);
    }
    changed
}

/// Info about an active agent path sent to the git worker.
struct GitWorkerPath {
    path: PathBuf,
    /// Whether this agent is stale (idle > threshold). Stale agents only
    /// get git status on the full sweep, not on every poll cycle.
    is_stale: bool,
}

/// Spawn a background thread that watches for git changes and updates the cache.
///
/// Uses the `notify` crate for OS-level filesystem event detection (FSEvents on macOS).
/// Watches .git internals and worktree roots for each active worktree. Events are
/// debounced per-worktree (300ms) before triggering `get_git_status()`. A fallback
/// sweep runs every 30s for edge cases where the watcher might miss events.
fn spawn_git_worker(
    term: Arc<AtomicBool>,
    dirty_flag: Arc<AtomicBool>,
) -> (GitCache, std::sync::mpsc::Sender<Vec<GitWorkerPath>>) {
    let cache: GitCache = Arc::new(Mutex::new(HashMap::new()));
    let cache_clone = cache.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<GitWorkerPath>>();

    thread::spawn(move || {
        // Filesystem event channel for notify
        let (fs_tx, fs_rx) = std::sync::mpsc::channel();
        let mut watcher: Option<notify::RecommendedWatcher> =
            match notify::RecommendedWatcher::new(fs_tx, notify::Config::default()) {
                Ok(w) => Some(w),
                Err(e) => {
                    tracing::warn!(
                        "filesystem watcher unavailable, falling back to polling: {}",
                        e
                    );
                    None
                }
            };

        let mut active_entries: Vec<GitWorkerPath> = Vec::new();
        // Maps: watched directory -> set of worktrees it covers
        let mut watch_to_worktrees: HashMap<PathBuf, HashSet<PathBuf>> = HashMap::new();
        // Maps: worktree path -> list of watched paths for it
        let mut worktree_watches: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        // Per-worktree: timestamp of last fs event (for debouncing)
        let mut pending_worktrees: HashMap<PathBuf, Instant> = HashMap::new();
        // Stale status per path (true = all agents at path are stale)
        let mut path_stale: HashMap<PathBuf, bool> = HashMap::new();
        // Track unique active paths for fallback polling
        let mut unique_active: Vec<PathBuf> = Vec::new();
        let mut last_full_sweep = Instant::now();
        // Watcher mode: 30s fallback sweep. Poll-only mode: 2s sweep interval.
        let full_sweep_interval = if watcher.is_some() {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(2)
        };
        let debounce_duration = Duration::from_millis(300);

        while !term.load(Ordering::Relaxed) {
            // Block on filesystem events (zero CPU when idle), or sleep briefly in poll mode
            if watcher.is_some() {
                let timeout = next_worker_timeout(
                    &pending_worktrees,
                    debounce_duration,
                    last_full_sweep,
                    full_sweep_interval,
                );
                match fs_rx.recv_timeout(timeout) {
                    Ok(Ok(event)) => {
                        for path in &event.paths {
                            for wt in find_worktrees_for_path(path, &watch_to_worktrees) {
                                pending_worktrees
                                    .entry(wt)
                                    .and_modify(|t| *t = Instant::now())
                                    .or_insert_with(Instant::now);
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("filesystem watch error: {}", e);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }

                // Drain any additional buffered events
                while let Ok(event_result) = fs_rx.try_recv() {
                    if let Ok(event) = event_result {
                        for path in &event.paths {
                            for wt in find_worktrees_for_path(path, &watch_to_worktrees) {
                                pending_worktrees
                                    .entry(wt)
                                    .and_modify(|t| *t = Instant::now())
                                    .or_insert_with(Instant::now);
                            }
                        }
                    }
                }
            } else {
                // Poll-only fallback: sleep until next sweep check
                let sleep = full_sweep_interval
                    .saturating_sub(last_full_sweep.elapsed())
                    .min(Duration::from_secs(1));
                thread::sleep(sleep);
            }

            // Check for path updates (non-blocking)
            let mut paths_changed = false;
            while let Ok(entries) = rx.try_recv() {
                active_entries = entries;
                paths_changed = true;
            }

            if paths_changed {
                // Deduplicate paths. A path is stale only if ALL agents at that path are stale.
                path_stale.clear();
                for entry in &active_entries {
                    let e = path_stale.entry(entry.path.clone()).or_insert(true);
                    if !entry.is_stale {
                        *e = false;
                    }
                }
                unique_active = path_stale.keys().cloned().collect();
                unique_active.sort();
                let unique_set: HashSet<PathBuf> = unique_active.iter().cloned().collect();

                if let Some(ref mut w) = watcher {
                    // Remove watches for worktrees no longer active
                    let removed: Vec<PathBuf> = worktree_watches
                        .keys()
                        .filter(|p| !unique_set.contains(*p))
                        .cloned()
                        .collect();
                    for path in &removed {
                        if let Some(watched_paths) = worktree_watches.remove(path) {
                            for wp in &watched_paths {
                                remove_worktree_watch(w, wp, path, &mut watch_to_worktrees);
                            }
                        }
                        pending_worktrees.remove(path);
                    }

                    // Add watches for new worktrees
                    for path in &unique_active {
                        if worktree_watches.contains_key(path) {
                            continue;
                        }
                        let watched = setup_worktree_watches(w, path, &mut watch_to_worktrees);
                        worktree_watches.insert(path.clone(), watched);
                        // Trigger immediate status fetch for new worktrees
                        pending_worktrees.insert(path.clone(), Instant::now() - debounce_duration);
                    }

                    // Prune cache for removed worktrees
                    if !removed.is_empty() {
                        if let Ok(mut c) = cache_clone.lock() {
                            c.retain(|p, _| unique_set.contains(p));
                        }
                        dirty_flag.store(true, Ordering::Relaxed);
                    }
                } else {
                    // Poll-only mode: just prune cache, no watches to manage
                    if let Ok(mut c) = cache_clone.lock() {
                        let before = c.len();
                        c.retain(|p, _| unique_set.contains(p));
                        if c.len() != before {
                            dirty_flag.store(true, Ordering::Relaxed);
                        }
                    }
                    // Trigger immediate fetch for new paths
                    for path in &unique_active {
                        if !cache_clone
                            .lock()
                            .ok()
                            .is_some_and(|c| c.contains_key(path))
                        {
                            pending_worktrees
                                .insert(path.clone(), Instant::now() - debounce_duration);
                        }
                    }
                }
            }

            // Process debounce-ready worktrees (skip stale ones, they only refresh on sweep)
            let now = Instant::now();
            let ready: Vec<PathBuf> = pending_worktrees
                .iter()
                .filter(|(_, last_event)| now.duration_since(**last_event) >= debounce_duration)
                .map(|(path, _)| path.clone())
                .collect();

            let mut any_changed = false;
            for path in &ready {
                pending_worktrees.remove(path);
                let is_stale = path_stale.get(path).copied().unwrap_or(false);
                if is_stale {
                    continue;
                }
                if refresh_git_status(path, &cache_clone) {
                    any_changed = true;
                }
            }

            // Fallback full sweep (30s with watcher, 2s without; includes stale worktrees)
            if last_full_sweep.elapsed() >= full_sweep_interval {
                last_full_sweep = Instant::now();
                let sweep_paths: Vec<PathBuf> = if watcher.is_some() {
                    worktree_watches.keys().cloned().collect()
                } else {
                    unique_active.clone()
                };
                for path in &sweep_paths {
                    if pending_worktrees.contains_key(path) {
                        continue;
                    }
                    if refresh_git_status(path, &cache_clone) {
                        any_changed = true;
                    }
                }
            }

            if any_changed {
                dirty_flag.store(true, Ordering::Relaxed);
            }
        }
    });

    (cache, tx)
}

/// Tracks pane content hashes to detect agents that stopped producing output.
///
/// For each agent in `Working` status, captures the last few lines of the pane,
/// hashes them, and records when the hash was first seen unchanged. If the hash
/// stays the same for longer than the timeout, the agent is considered interrupted.
///
/// Once interrupted, the state is sticky: it only clears when the agent's state
/// is updated via RPC (detected by `updated_ts` changing) or when the agent
/// leaves Working status. Pane content changes (cursor movement, typing) do not
/// clear the interrupted state.
struct InactivityTracker {
    /// pane_id -> (content_hash, first_seen_at, updated_ts at recording time)
    entries: HashMap<String, (u64, Instant, u64)>,
    /// pane_id -> updated_ts at the time interruption was confirmed.
    /// Cleared when updated_ts changes (agent sent a new RPC status update).
    confirmed: HashMap<String, u64>,
    /// How long content must be unchanged before marking as interrupted.
    timeout: Duration,
}

impl InactivityTracker {
    fn new(timeout: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            confirmed: HashMap::new(),
            timeout,
        }
    }

    /// Check all working agents for inactivity. Returns the set of pane IDs
    /// that appear interrupted (content unchanged for longer than timeout).
    fn check(
        &mut self,
        agents: &[crate::multiplexer::AgentPane],
        mux: &dyn crate::multiplexer::Multiplexer,
    ) -> HashSet<String> {
        self.check_with(agents, Instant::now(), |pane_id| {
            mux.capture_pane(pane_id, 5)
        })
    }

    /// Testable core: accepts an explicit `now` and a capture function.
    fn check_with(
        &mut self,
        agents: &[crate::multiplexer::AgentPane],
        now: Instant,
        capture: impl Fn(&str) -> Option<String>,
    ) -> HashSet<String> {
        use std::hash::{Hash, Hasher};

        // Build lookup of working agents
        let working: HashMap<&str, &crate::multiplexer::AgentPane> = agents
            .iter()
            .filter(|a| a.status == Some(crate::multiplexer::AgentStatus::Working))
            .map(|a| (a.pane_id.as_str(), a))
            .collect();

        // Remove entries for agents no longer in Working status
        self.entries
            .retain(|id, _| working.contains_key(id.as_str()));
        self.confirmed
            .retain(|id, _| working.contains_key(id.as_str()));

        // Clear interrupted state if the agent's state was updated via RPC
        // (updated_ts changed since we confirmed the interruption).
        // Collect resumed pane IDs first, then clear their entries for a fresh
        // inactivity window.
        let resumed: Vec<String> = self
            .confirmed
            .iter()
            .filter(|(id, confirmed_ts)| {
                working
                    .get(id.as_str())
                    .is_some_and(|a| a.updated_ts.unwrap_or(0) > **confirmed_ts)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in &resumed {
            self.confirmed.remove(id);
            self.entries.remove(id);
        }

        for (pane_id, agent) in &working {
            // Already confirmed interrupted - skip capture
            if self.confirmed.contains_key(*pane_id) {
                continue;
            }

            let Some(raw) = capture(pane_id) else {
                continue;
            };

            // Strip ANSI escapes and normalize whitespace for stable hashing
            let stripped = console::strip_ansi_codes(&raw);
            let normalized = stripped.trim();

            let mut hasher = std::hash::DefaultHasher::new();
            normalized.hash(&mut hasher);
            let hash = hasher.finish();

            let current_rpc = agent.updated_ts.unwrap_or(0);

            match self.entries.get(*pane_id) {
                Some(&(prev_hash, first_seen, prev_rpc))
                    if prev_hash == hash && prev_rpc == current_rpc =>
                {
                    // Same content and same RPC state: check timeout
                    if now.duration_since(first_seen) >= self.timeout {
                        self.confirmed.insert(pane_id.to_string(), current_rpc);
                    }
                }
                _ => {
                    // Content changed or RPC updated: reset inactivity window
                    self.entries
                        .insert(pane_id.to_string(), (hash, now, current_rpc));
                }
            }
        }

        self.confirmed.keys().cloned().collect()
    }
}

/// Run the sidebar daemon (headless, no TUI).
pub fn run() -> Result<()> {
    let mux = create_backend(detect_backend());
    let instance_id = mux.instance_id();
    let config = Config::load(None)?;
    let status_icons = config.status_icons.clone();

    // Signal handlers for clean shutdown and dirty notification
    let term = Arc::new(AtomicBool::new(false));
    let dirty_flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, term.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGUSR1, dirty_flag.clone())?;

    let sock_path = socket_path(&instance_id);
    let _ = std::fs::remove_file(&sock_path); // Clean stale
    let server = SocketServer::bind(&sock_path, dirty_flag.clone())?;

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

    let mut inactivity_tracker = InactivityTracker::new(Duration::from_secs(10));
    let mut last_interrupted: HashSet<String> = HashSet::new();
    let mut last_runtime_write = Instant::now();
    let backend_name = mux.name().to_string();

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

            if let Some(mut snapshot) = try_build_snapshot(&mux, &status_icons, &config, &git_cache)
            {
                // Detect interrupted agents (working but no pane output change)
                let interrupted = inactivity_tracker.check(&snapshot.agents, mux.as_ref());
                snapshot.interrupted_pane_ids = interrupted.clone();

                // Reset status_ts for agents that just resumed from interruption
                // so their timer starts fresh.
                if !last_interrupted.is_empty() {
                    let resumed: Vec<&str> = last_interrupted
                        .iter()
                        .filter(|id| !interrupted.contains(*id))
                        .map(|s| s.as_str())
                        .collect();
                    if !resumed.is_empty()
                        && let Ok(store) = StateStore::new()
                    {
                        let now_ts = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        for pane_id in resumed {
                            let pane_key = crate::state::PaneKey {
                                backend: backend_name.clone(),
                                instance: instance_id.clone(),
                                pane_id: pane_id.to_string(),
                            };
                            if let Ok(Some(mut state)) = store.get_agent(&pane_key) {
                                state.status_ts = Some(now_ts);
                                let _ = store.upsert_agent(&state);
                            }
                        }
                    }
                }

                // Persist to runtime file so dashboard can read it.
                // Write on change, or periodically to keep updated_ts fresh
                // (dashboard ignores files older than 15s).
                let set_changed = interrupted != last_interrupted;
                let heartbeat_due = last_runtime_write.elapsed() >= Duration::from_secs(10);
                if set_changed || heartbeat_due {
                    last_interrupted = interrupted;
                    last_runtime_write = Instant::now();
                    if let Ok(store) = StateStore::new() {
                        let now_ts = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        let runtime = crate::state::RuntimeState {
                            interrupted_pane_ids: last_interrupted.clone(),
                            updated_ts: now_ts,
                        };
                        let _ = store.write_runtime(&backend_name, &instance_id, &runtime);
                    }
                }

                // Update git worker with current agent paths and stale status.
                // Stale agents (idle > 1 hour) are polled less frequently.
                let now_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let stale_threshold = 60 * 60; // 1 hour, matches sidebar UI
                let entries: Vec<GitWorkerPath> = snapshot
                    .agents
                    .iter()
                    .map(|a| GitWorkerPath {
                        path: a.path.clone(),
                        is_stale: a
                            .status_ts
                            .map(|ts| now_secs.saturating_sub(ts) > stale_threshold)
                            .unwrap_or(false),
                    })
                    .collect();
                let _ = git_path_tx.send(entries);

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
    if let Ok(store) = StateStore::new() {
        store.delete_runtime(&backend_name, &instance_id);
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiplexer::{AgentPane, AgentStatus};
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn working_agent(pane_id: &str, updated_ts: u64) -> AgentPane {
        AgentPane {
            session: String::new(),
            window_name: String::new(),
            pane_id: pane_id.to_string(),
            window_id: String::new(),
            path: PathBuf::new(),
            pane_title: None,
            status: Some(AgentStatus::Working),
            status_ts: Some(100),
            updated_ts: Some(updated_ts),
        }
    }

    fn done_agent(pane_id: &str) -> AgentPane {
        AgentPane {
            status: Some(AgentStatus::Done),
            ..working_agent(pane_id, 1)
        }
    }

    #[test]
    fn no_interruption_before_timeout() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // First check: records the hash
        let result = tracker.check_with(&agents, t0, |_| Some("hello".into()));
        assert!(result.is_empty());

        // 5s later, same content: not yet interrupted
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(5), |_| {
            Some("hello".into())
        });
        assert!(result.is_empty());
    }

    #[test]
    fn interruption_after_timeout() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // First check records hash
        tracker.check_with(&agents, t0, |_| Some("hello".into()));

        // 11s later, same content: interrupted
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(11), |_| {
            Some("hello".into())
        });
        assert!(result.contains("%1"));
    }

    #[test]
    fn changing_content_resets_window() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // First check
        tracker.check_with(&agents, t0, |_| Some("hello".into()));

        // 8s later, content changes: resets the window
        tracker.check_with(&agents, t0 + Duration::from_secs(8), |_| {
            Some("world".into())
        });

        // 5s after the change (13s total): not interrupted (only 5s since reset)
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(13), |_| {
            Some("world".into())
        });
        assert!(result.is_empty());

        // 11s after the change (19s total): now interrupted
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(19), |_| {
            Some("world".into())
        });
        assert!(result.contains("%1"));
    }

    #[test]
    fn sticky_despite_content_change() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // Become interrupted
        tracker.check_with(&agents, t0, |_| Some("hello".into()));
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(11), |_| {
            Some("hello".into())
        });
        assert!(result.contains("%1"));

        // Content changes (user typing): still interrupted
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(12), |_| {
            Some("user typed something".into())
        });
        assert!(result.contains("%1"));
    }

    #[test]
    fn clears_on_updated_ts_change() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // Become interrupted
        tracker.check_with(&agents, t0, |_| Some("hello".into()));
        tracker.check_with(&agents, t0 + Duration::from_secs(11), |_| {
            Some("hello".into())
        });

        // Agent sends new RPC (updated_ts changes): clears interrupted
        let resumed_agents = vec![working_agent("%1", 2)];
        let result = tracker.check_with(&resumed_agents, t0 + Duration::from_secs(12), |_| {
            Some("hello".into())
        });
        assert!(result.is_empty());
    }

    #[test]
    fn fresh_window_after_resume() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // Become interrupted
        tracker.check_with(&agents, t0, |_| Some("hello".into()));
        tracker.check_with(&agents, t0 + Duration::from_secs(11), |_| {
            Some("hello".into())
        });

        // Resume (updated_ts changes) at t=12s
        let resumed = vec![working_agent("%1", 2)];
        tracker.check_with(&resumed, t0 + Duration::from_secs(12), |_| {
            Some("hello".into())
        });

        // 5s after resume (t=17s): same content but not interrupted yet (fresh window)
        let result = tracker.check_with(&resumed, t0 + Duration::from_secs(17), |_| {
            Some("hello".into())
        });
        assert!(result.is_empty());

        // 11s after resume (t=23s): now interrupted again
        let result = tracker.check_with(&resumed, t0 + Duration::from_secs(23), |_| {
            Some("hello".into())
        });
        assert!(result.contains("%1"));
    }

    #[test]
    fn non_working_agents_ignored() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![done_agent("%1")];
        let t0 = Instant::now();

        tracker.check_with(&agents, t0, |_| Some("hello".into()));
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(11), |_| {
            Some("hello".into())
        });
        assert!(result.is_empty());
    }

    #[test]
    fn leaves_working_clears_tracking() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let working = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // Become interrupted
        tracker.check_with(&working, t0, |_| Some("hello".into()));
        tracker.check_with(&working, t0 + Duration::from_secs(11), |_| {
            Some("hello".into())
        });

        // Agent transitions to Done
        let done = vec![done_agent("%1")];
        let result = tracker.check_with(&done, t0 + Duration::from_secs(12), |_| {
            Some("hello".into())
        });
        assert!(result.is_empty());

        // Comes back as Working: starts fresh
        let working_again = vec![working_agent("%1", 3)];
        let result = tracker.check_with(&working_again, t0 + Duration::from_secs(13), |_| {
            Some("hello".into())
        });
        assert!(result.is_empty()); // just recorded, not yet timed out
    }

    #[test]
    fn capture_failure_skips_pane() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // Capture fails: no entry recorded
        tracker.check_with(&agents, t0, |_| None);
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(11), |_| None);
        assert!(result.is_empty());
    }

    #[test]
    fn multiple_agents_tracked_independently() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1), working_agent("%2", 1)];
        let t0 = Instant::now();

        let content = RefCell::new(HashMap::from([
            ("%1".to_string(), "static".to_string()),
            ("%2".to_string(), "changing".to_string()),
        ]));

        // First check
        tracker.check_with(&agents, t0, |id| content.borrow().get(id).cloned());

        // Change %2's content at 5s
        content
            .borrow_mut()
            .insert("%2".to_string(), "new output".into());
        tracker.check_with(&agents, t0 + Duration::from_secs(5), |id| {
            content.borrow().get(id).cloned()
        });

        // At 11s: %1 is interrupted (11s unchanged), %2 is not (only 6s since change)
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(11), |id| {
            content.borrow().get(id).cloned()
        });
        assert_eq!(result, HashSet::from(["%1".to_string()]));
    }

    #[test]
    fn rpc_update_before_timeout_resets_window() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let t0 = Instant::now();

        // Start tracking
        tracker.check_with(&[working_agent("%1", 1)], t0, |_| Some("hello".into()));

        // Agent sends RPC at 5s (updated_ts changes) but content unchanged
        tracker.check_with(
            &[working_agent("%1", 2)],
            t0 + Duration::from_secs(5),
            |_| Some("hello".into()),
        );

        // At 11s: only 6s since RPC update, should NOT be interrupted
        let result = tracker.check_with(
            &[working_agent("%1", 2)],
            t0 + Duration::from_secs(11),
            |_| Some("hello".into()),
        );
        assert!(result.is_empty());

        // At 16s: 11s since RPC update, now interrupted
        let result = tracker.check_with(
            &[working_agent("%1", 2)],
            t0 + Duration::from_secs(16),
            |_| Some("hello".into()),
        );
        assert_eq!(result, HashSet::from(["%1".to_string()]));
    }

    #[test]
    fn interruption_at_exact_timeout() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        tracker.check_with(&agents, t0, |_| Some("hello".into()));
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(10), |_| {
            Some("hello".into())
        });
        assert_eq!(result, HashSet::from(["%1".to_string()]));
    }

    #[test]
    fn ansi_and_whitespace_normalized() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // Plain text first
        tracker.check_with(&agents, t0, |_| Some("hello\n".into()));

        // Same text wrapped in ANSI codes + trailing whitespace: should hash the same
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(11), |_| {
            Some("\x1b[31mhello\x1b[0m   ".into())
        });
        assert_eq!(result, HashSet::from(["%1".to_string()]));
    }

    #[test]
    fn capture_failure_does_not_create_baseline() {
        let mut tracker = InactivityTracker::new(Duration::from_secs(10));
        let agents = vec![working_agent("%1", 1)];
        let t0 = Instant::now();

        // Capture fails: no baseline recorded
        tracker.check_with(&agents, t0, |_| None);

        // Capture succeeds later: this is the first successful capture, not a timeout
        let result = tracker.check_with(&agents, t0 + Duration::from_secs(11), |_| {
            Some("hello".into())
        });
        assert!(result.is_empty());
    }
}
