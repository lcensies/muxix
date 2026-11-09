//! Muxix daemon: single process that watches tmux and pushes snapshots to
//! clients. Event-driven: agent status hooks and tmux lifecycle hooks SIGUSR1
//! us; a slow timer backstops stall detection and lost hook events.

use anyhow::Result;
use ignore::gitignore::Gitignore;
use notify::{RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::cmd::Cmd;
use crate::config::{Config, SidebarPosition};
use crate::git::GitStatus;
use crate::github::PrSummary;
use crate::multiplexer::{AgentPane, AgentStatus, Multiplexer, create_backend, detect_backend};
use crate::state::StateStore;

use super::app::SidebarLayoutMode;
use super::snapshot::{PrPathEntry, build_snapshot};

/// How often the sidebar polls ADE runtimes. Each poll spawns a subprocess per
/// configured ADE, so it is deliberately far slower than the refresh tick; the
/// previous result is reused in between.
const ADE_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Drift insurance: full refresh cadence when no agent is working. Real
/// updates arrive event-driven (agent status hooks and tmux lifecycle hooks
/// SIGUSR1 us); this sweep only catches events lost while the daemon was
/// down and hookless foreign agents.
const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// Refresh cadence while at least one agent is Working. Stall detection
/// needs sampling — an agent going silent produces no event.
const WORKING_TICK_INTERVAL: Duration = Duration::from_secs(5);

/// How long the daemon stays up after its last client, when no background job
/// needs it. Also a loop deadline: the wait must not outlast it.
const IDLE_EXIT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a working agent's pane content must be unchanged (with no RPC
/// heartbeat) before it is marked interrupted. Also gates pane captures:
/// agents with a heartbeat fresher than this cannot be marked interrupted,
/// so their capture is skipped entirely.
const INACTIVITY_TIMEOUT: Duration = Duration::from_secs(10);

/// Agents owned by non-local runtimes, cached between polls.
///
/// A failing or unavailable runtime leaves the previous list in place, so one
/// bad call (the manager's daemon restarting, say) does not make its sessions
/// flicker out of the sidebar. The failure is logged once per distinct message.
struct AdeAgents {
    polled: Option<Instant>,
    agents: Vec<AgentPane>,
    last_problem: Option<String>,
}

impl AdeAgents {
    fn new() -> Self {
        Self {
            polled: None,
            agents: Vec::new(),
            last_problem: None,
        }
    }

    fn poll(&mut self, cfg: &Config) -> Vec<AgentPane> {
        if let Some(at) = self.polled
            && at.elapsed() < ADE_POLL_INTERVAL
        {
            return self.agents.clone();
        }
        let (panes, problems) = crate::agent::runtime::registry::foreign_agent_panes(cfg);
        self.apply(Instant::now(), panes, problems)
    }

    fn apply(
        &mut self,
        now: Instant,
        panes: Vec<AgentPane>,
        problems: Vec<(String, String)>,
    ) -> Vec<AgentPane> {
        self.polled = Some(now);
        if problems.is_empty() {
            self.last_problem = None;
            self.agents = panes;
        } else {
            let text = problems
                .iter()
                .map(|(name, why)| format!("{name}: {why}"))
                .collect::<Vec<_>>()
                .join("; ");
            if self.last_problem.as_deref() != Some(&text) {
                tracing::warn!(
                    problems = %text,
                    kept = self.agents.len(),
                    "agent manager listing failed; keeping the last good list"
                );
                self.last_problem = Some(text);
            }
        }
        self.agents.clone()
    }
}

/// How long the main loop may block: the soonest deadline it must wake for.
///
/// Signals arrive on the wake channel, so nothing here is a flag poll — an
/// idle daemon sleeps until its next sweep.
fn next_loop_wait(
    since_refresh: Duration,
    refresh_interval: Duration,
    idle_exit_in: Option<Duration>,
    health_log_in: Duration,
) -> Duration {
    let wait = refresh_interval
        .saturating_sub(since_refresh)
        .min(health_log_in);
    match idle_exit_in {
        Some(exit_in) => wait.min(exit_in),
        None => wait,
    }
}

/// The project config governing a path: the directory it lives in (what the
/// config watcher needs) and the `main_branch` it sets (what git stats need).
#[derive(Clone, Debug, PartialEq, Eq)]
struct GoverningConfig {
    dir: PathBuf,
    main_branch: Option<String>,
}

/// Maps each agent's working directory to the project config that governs it,
/// caching both hits and misses.
///
/// `find_project_config` walks ancestors and shells out to git, and most
/// worktrees have no `.muxix.yaml` at all, so a miss must be cached too or
/// every agent costs a walk on every tick.
struct ProjectConfigs {
    cache: HashMap<PathBuf, Option<GoverningConfig>>,
    cached_at: Instant,
    seen_version: u64,
}

impl ProjectConfigs {
    fn new(version: u64, now: Instant) -> Self {
        Self {
            cache: HashMap::new(),
            cached_at: now,
            seen_version: version,
        }
    }

    /// The config governing each of `paths`, resolving only what is not already
    /// cached. The returned map is pruned to exactly `paths`.
    ///
    /// The whole cache is dropped when the config version moves (a `.muxix.yaml`
    /// the watcher can see was added, changed or removed) or once per sweep (to
    /// catch the directories it cannot see).
    fn resolve_all(
        &mut self,
        paths: &[PathBuf],
        version_now: u64,
        now: Instant,
        resolve: &dyn Fn(&Path) -> Option<GoverningConfig>,
    ) -> &HashMap<PathBuf, Option<GoverningConfig>> {
        let live: HashSet<&PathBuf> = paths.iter().collect();
        self.cache.retain(|p, _| live.contains(p));

        if version_now != self.seen_version || now.duration_since(self.cached_at) >= SWEEP_INTERVAL
        {
            self.seen_version = version_now;
            self.cached_at = now;
            self.cache.clear();
        }

        for path in paths {
            self.cache
                .entry(path.clone())
                .or_insert_with(|| resolve(path));
        }
        &self.cache
    }
}

/// The config dirs to watch, deduped.
fn config_dirs_of(governing: &HashMap<PathBuf, Option<GoverningConfig>>) -> HashSet<PathBuf> {
    governing
        .values()
        .flatten()
        .map(|c| c.dir.clone())
        .collect()
}

/// The `main_branch` the project config governing `path` sets, falling back to
/// the daemon's global config.
fn main_branch_for(
    governing: &HashMap<PathBuf, Option<GoverningConfig>>,
    path: &PathBuf,
    global: Option<&str>,
) -> Option<String> {
    governing
        .get(path)
        .and_then(|c| c.as_ref())
        .and_then(|c| c.main_branch.clone())
        .or_else(|| global.map(str::to_string))
        .filter(|b| !b.is_empty())
}

/// Resolve the project config governing `path`, reading only `main_branch` out
/// of it.
///
/// One daemon serves worktrees from many projects, so there is no single
/// `main_branch`; a full `Config::load` per path per tick is the cost
/// `ProjectConfigs` exists to avoid.
///
/// shortcut: reads the file's top-level `main_branch` only, so a `main_branch`
/// set under a `profiles:` overlay is missed; revisit if the daemon ever needs
/// profile-aware config.
fn governing_config_of(path: &Path) -> Option<GoverningConfig> {
    let loc = crate::config::find_project_config(path).ok().flatten()?;
    let main_branch = std::fs::read_to_string(&loc.config_path)
        .ok()
        .and_then(|text| serde_yaml::from_str::<serde_yaml::Value>(&text).ok())
        .and_then(|v| v.get("main_branch")?.as_str().map(str::to_string));
    Some(GoverningConfig {
        dir: loc.config_dir,
        main_branch,
    })
}

#[cfg(test)]
mod project_configs_tests {
    use super::*;
    use std::cell::RefCell;

    fn governing(dir: &str, main_branch: Option<&str>) -> Option<GoverningConfig> {
        Some(GoverningConfig {
            dir: PathBuf::from(dir),
            main_branch: main_branch.map(str::to_string),
        })
    }

    /// Resolver that records every path it was asked about.
    fn counting(
        hits: &RefCell<Vec<PathBuf>>,
        answer: Option<GoverningConfig>,
    ) -> impl Fn(&Path) -> Option<GoverningConfig> + '_ {
        move |p: &Path| {
            hits.borrow_mut().push(p.to_path_buf());
            answer.clone()
        }
    }

    #[test]
    fn a_worktree_with_no_project_config_is_resolved_once_not_once_per_tick() {
        let t0 = Instant::now();
        let mut cfgs = ProjectConfigs::new(0, t0);
        let hits = RefCell::new(Vec::new());
        let resolve = counting(&hits, None);
        let paths = vec![PathBuf::from("/wt/a")];

        for i in 0..10 {
            let out = cfgs.resolve_all(&paths, 0, t0 + Duration::from_secs(i), &resolve);
            assert!(
                config_dirs_of(out).is_empty(),
                "a miss contributes no config dir"
            );
        }

        assert_eq!(
            hits.borrow().len(),
            1,
            "the miss must be cached, not re-walked"
        );
    }

    #[test]
    fn a_config_appearing_later_is_picked_up_when_the_version_moves() {
        let t0 = Instant::now();
        let mut cfgs = ProjectConfigs::new(0, t0);
        let paths = vec![PathBuf::from("/wt/a")];

        let misses = RefCell::new(Vec::new());
        assert!(
            config_dirs_of(cfgs.resolve_all(&paths, 0, t0, &counting(&misses, None))).is_empty()
        );

        // The watcher saw a .muxix.yaml appear and bumped the version.
        let found = RefCell::new(Vec::new());
        let out = cfgs.resolve_all(
            &paths,
            1,
            t0 + Duration::from_secs(1),
            &counting(&found, governing("/wt/a", None)),
        );
        assert_eq!(found.borrow().len(), 1, "a version bump must re-resolve");
        assert_eq!(config_dirs_of(out), HashSet::from([PathBuf::from("/wt/a")]));
    }

    #[test]
    fn the_sweep_re_resolves_what_the_watcher_cannot_see() {
        let t0 = Instant::now();
        let mut cfgs = ProjectConfigs::new(0, t0);
        let paths = vec![PathBuf::from("/wt/a")];
        let hits = RefCell::new(Vec::new());
        let resolve = counting(&hits, None);

        cfgs.resolve_all(&paths, 0, t0, &resolve);
        cfgs.resolve_all(
            &paths,
            0,
            t0 + SWEEP_INTERVAL - Duration::from_millis(1),
            &resolve,
        );
        assert_eq!(hits.borrow().len(), 1, "still inside the sweep window");

        cfgs.resolve_all(&paths, 0, t0 + SWEEP_INTERVAL, &resolve);
        assert_eq!(hits.borrow().len(), 2, "the sweep expires the cache");
    }

    #[test]
    fn a_departed_agents_entry_is_dropped() {
        let t0 = Instant::now();
        let mut cfgs = ProjectConfigs::new(0, t0);
        let hits = RefCell::new(Vec::new());
        let resolve = counting(&hits, None);

        cfgs.resolve_all(&[PathBuf::from("/wt/a")], 0, t0, &resolve);
        // /wt/a's agent is gone; only /wt/b remains.
        let out = cfgs.resolve_all(&[PathBuf::from("/wt/b")], 0, t0, &resolve);
        assert_eq!(out.len(), 1);
        assert!(out.contains_key(&PathBuf::from("/wt/b")));
    }

    #[test]
    fn two_agents_sharing_a_config_dir_yield_one_entry() {
        let t0 = Instant::now();
        let mut cfgs = ProjectConfigs::new(0, t0);
        let hits = RefCell::new(Vec::new());
        let resolve = counting(&hits, governing("/repo", None));

        let out = cfgs.resolve_all(
            &[PathBuf::from("/repo/a"), PathBuf::from("/repo/b")],
            0,
            t0,
            &resolve,
        );
        assert_eq!(hits.borrow().len(), 2, "each path is resolved");
        assert_eq!(
            config_dirs_of(out),
            HashSet::from([PathBuf::from("/repo")]),
            "deduped by dir"
        );
    }

    #[test]
    fn the_main_branch_a_worktree_is_governed_by_costs_one_resolve() {
        let t0 = Instant::now();
        let mut cfgs = ProjectConfigs::new(0, t0);
        let hits = RefCell::new(Vec::new());
        let resolve = counting(&hits, governing("/repo", Some("trunk")));
        let paths = vec![PathBuf::from("/repo/a")];

        for i in 0..10 {
            let out = cfgs.resolve_all(&paths, 0, t0 + Duration::from_secs(i), &resolve);
            assert_eq!(
                main_branch_for(out, &paths[0], Some("global")).as_deref(),
                Some("trunk"),
                "the project config outranks the global one"
            );
        }
        assert_eq!(
            hits.borrow().len(),
            1,
            "the steady-state tick must not re-resolve"
        );
    }

    #[test]
    fn a_path_with_no_project_main_branch_falls_back_to_the_global_one() {
        let t0 = Instant::now();
        let mut cfgs = ProjectConfigs::new(0, t0);
        let hits = RefCell::new(Vec::new());
        let unset = PathBuf::from("/repo/a");
        let out = cfgs.resolve_all(
            std::slice::from_ref(&unset),
            0,
            t0,
            &counting(&hits, governing("/repo", None)),
        );
        assert_eq!(
            main_branch_for(out, &unset, Some("trunk")).as_deref(),
            Some("trunk")
        );

        let missing = PathBuf::from("/wt/b");
        let out = cfgs.resolve_all(
            std::slice::from_ref(&missing),
            0,
            t0,
            &counting(&hits, None),
        );
        assert_eq!(
            main_branch_for(out, &missing, Some("trunk")).as_deref(),
            Some("trunk"),
            "no project config at all still honours the global main_branch"
        );
        assert_eq!(
            main_branch_for(out, &missing, None),
            None,
            "nothing configured leaves the base to git's own detection"
        );
        assert_eq!(
            main_branch_for(out, &missing, Some("")),
            None,
            "an empty main_branch is not a base"
        );
    }
}

/// Compute socket path from instance_id.
pub fn socket_path(instance_id: &str) -> PathBuf {
    let safe_id = instance_id.replace(['/', '\\'], "-");
    std::env::temp_dir().join(format!("muxix-daemon-{}.sock", safe_id))
}

/// Result of a batched tmux query.
struct TmuxState {
    window_statuses: HashMap<String, Option<String>>,
    active_windows: HashSet<(String, String)>,
    pane_window_ids: HashMap<String, String>,
    active_pane_ids: HashSet<String>,
    window_pane_counts: HashMap<String, usize>,
    /// `list-panes` itself failed: the tmux server has most likely exited.
    server_gone: bool,
}

/// Query all sidebar-relevant tmux state in a single command.
fn query_tmux_state() -> TmuxState {
    let format = "#{pane_id}\t#{session_name}\t#{window_id}\t#{@muxix_pane_status}\t#{window_active}\t#{session_attached}\t#{pane_active}";
    let output = Cmd::new("tmux")
        .args(&["list-panes", "-a", "-F", format])
        .run_and_capture_stdout();
    // A failed list-panes almost always means the server is gone (tmux exits
    // with its last session), which is the daemon's cue to exit too.
    let server_gone = output.is_err();
    let output = output.unwrap_or_default();

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
        server_gone,
    }
}

/// A connected client. Snapshot pushes and request replies are written from
/// different threads, so the write half is shared under a mutex: two frames
/// must never interleave on one socket.
type Client = Arc<Mutex<UnixStream>>;

/// What a request handler needs to answer `GetGitStatuses`.
#[derive(Clone)]
struct GitRequests {
    cache: GitCache,
    prioritise: PrioritiseQueue,
}

/// Unix socket server for broadcasting snapshots to clients.
struct SocketServer {
    clients: Arc<Mutex<Vec<Client>>>,
    /// Cached last broadcast payload (length-prefixed) for immediate delivery to new clients.
    cached_payload: Arc<Mutex<Vec<u8>>>,
    /// The snapshot the clients already hold, so an unchanged tick sends
    /// nothing. Compared as a value, not as serialized bytes: hash-map
    /// iteration order makes two serializations of equal state differ.
    last_sent: Mutex<Option<super::snapshot::SidebarSnapshot>>,
}

impl SocketServer {
    fn bind(path: &Path, git: GitRequests) -> std::io::Result<Self> {
        let listener = UnixListener::bind(path)?;
        // Restrict socket to owner only (prevent other local users from reading snapshots)
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        // The listener stays blocking: the accept thread has nothing else to
        // do, and process exit is what ends it. Polling it would be a second
        // always-on wakeup for no gain.
        let clients: Arc<Mutex<Vec<Client>>> = Arc::new(Mutex::new(Vec::new()));
        let clients_clone = clients.clone();
        let cached_payload: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
        let cached_clone = cached_payload.clone();

        thread::spawn(move || {
            while let Ok((mut stream, _)) = listener.accept() {
                let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                // Send cached snapshot immediately so the client doesn't
                // wait for the next tick (no dirty_flag needed).
                // Clone under lock and drop before the blocking write to
                // avoid holding the mutex during I/O.
                let payload = cached_clone.lock().ok().map(|p| p.clone());
                let cache_ok = match payload {
                    Some(ref p) if !p.is_empty() => stream.write_all(p).is_ok(),
                    _ => true,
                };
                if !cache_ok {
                    continue;
                }
                let Ok(reader) = stream.try_clone() else {
                    continue;
                };
                let client: Client = Arc::new(Mutex::new(stream));
                {
                    let mut clients = clients_clone.lock().unwrap();
                    clients.push(client.clone());
                    tracing::debug!(clients = clients.len(), "sidebar client connected");
                }
                let git = git.clone();
                thread::spawn(move || serve_requests(reader, &client, &git));
            }
        });

        Ok(Self {
            clients,
            cached_payload,
            last_sent: Mutex::new(None),
        })
    }

    /// Push `snapshot` to every client unless it is the one they already hold.
    ///
    /// Returns whether anything was sent. Clients redraw only from a pushed
    /// snapshot, so re-pushing an identical one is pure cost; a skipped push
    /// still reaps closed connections, because the idle exit watches the
    /// client count.
    fn broadcast(&self, snapshot: &super::snapshot::SidebarSnapshot) -> bool {
        if let Ok(last) = self.last_sent.lock()
            && last.as_ref() == Some(snapshot)
        {
            self.prune_dead_clients();
            return false;
        }
        let data = match serde_json::to_vec(snapshot) {
            Ok(d) => d,
            Err(e) => {
                // Don't broadcast (or cache) a zero-length frame: a new client
                // would receive an empty snapshot instead of stale-but-valid
                // data, and existing clients would get a bogus 0-byte payload.
                tracing::warn!(error = %e, "sidebar broadcast: failed to serialize snapshot; skipping");
                return false;
            }
        };
        let len = (data.len() as u32).to_be_bytes();

        // Cache the length-prefixed payload for new client connections
        if let Ok(mut cached) = self.cached_payload.lock() {
            cached.clear();
            cached.extend_from_slice(&len);
            cached.extend_from_slice(&data);
        }

        // Take clients out of mutex to avoid holding lock during writes
        let mut clients = std::mem::take(&mut *self.clients.lock().unwrap());
        let before = clients.len();
        clients.retain(|client| match client.lock() {
            Ok(mut stream) => {
                if stream.write_all(&len).is_ok() && stream.write_all(&data).is_ok() {
                    return true;
                }
                evict(&stream);
                false
            }
            Err(_) => false,
        });
        let dropped = before.saturating_sub(clients.len());
        if dropped > 0 {
            tracing::info!(
                dropped,
                remaining = clients.len(),
                payload_bytes = data.len(),
                "sidebar broadcast: clients disconnected"
            );
        }
        // Merge surviving clients back (append to preserve any new connections accepted during writes)
        self.clients.lock().unwrap().append(&mut clients);
        if let Ok(mut last) = self.last_sent.lock() {
            *last = Some(snapshot.clone());
        }
        true
    }

    /// Drop clients that have closed their end. See [`client_is_gone`].
    ///
    /// A broadcast reaps them as a side effect of failing to write; a tick
    /// that skips the broadcast still has to, because the daemon's idle exit
    /// is driven by the client count.
    fn prune_dead_clients(&self) {
        let mut clients = std::mem::take(&mut *self.clients.lock().unwrap());
        let before = clients.len();
        clients.retain(|client| match client.lock() {
            Ok(stream) => {
                if !client_is_gone(&stream) {
                    return true;
                }
                evict(&stream);
                false
            }
            Err(_) => false,
        });
        let dropped = before.saturating_sub(clients.len());
        if dropped > 0 {
            tracing::info!(
                dropped,
                remaining = clients.len(),
                "sidebar: clients disconnected"
            );
        }
        self.clients.lock().unwrap().append(&mut clients);
    }

    fn client_count(&self) -> usize {
        self.clients.lock().unwrap().len()
    }
}

/// Drop a client for good: shut the socket down rather than waiting for the
/// last `Arc` to go.
///
/// The accepted stream has a 100ms write timeout, so a stalled peer can leave
/// a half-written frame behind; the shutdown stops the client from reading the
/// remains as framing, hands it its EOF, and unparks the thread sitting in
/// [`serve_requests`] (which holds an `Arc`, so the drop alone would not).
fn evict(stream: &std::os::unix::net::UnixStream) {
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

/// Has this client closed its end?
///
/// Clients send requests, so readable does not mean gone — only a zero-length
/// read does. A non-destructive peek distinguishes the two: 0 is the peer's
/// orderly shutdown, `EAGAIN` is a live idle client, and a pending request
/// byte is a live busy one. `recv` is not restarted after a signal on Linux
/// whatever `SA_RESTART` says, and the daemon is SIGUSR1'd by every
/// agent-status hook, so `EINTR` has to be retried rather than counted as a
/// hangup.
///
/// Queued bytes do not mean the peer is still there: a desynced stream (see
/// [`serve_requests`]) leaves them unread forever, so a corpse would peek as a
/// busy client and keep the daemon's idle exit from ever firing. `poll` still
/// separates the two — it reports `POLLHUP` for a closed peer whether or not
/// data is pending — so the peek only decides between idle and busy.
fn client_is_gone(stream: &std::os::unix::net::UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut byte = 0u8;
    loop {
        let n = unsafe {
            libc::recv(
                stream.as_raw_fd(),
                std::ptr::from_mut(&mut byte).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        if n == 0 {
            return true;
        }
        if n > 0 {
            return peer_hung_up(stream);
        }
        return match std::io::Error::last_os_error().kind() {
            std::io::ErrorKind::Interrupted => continue,
            std::io::ErrorKind::WouldBlock => false,
            _ => true,
        };
    }
}

/// Does `poll` report this socket's peer as gone?
///
/// Only the hangup bits are read; `POLLIN` is requested because macOS turns
/// `events` into kqueue filters and registers none at all for `events = 0`, so
/// a hangup there would never be reported.
fn peer_hung_up(stream: &std::os::unix::net::UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    loop {
        let mut fd = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut fd, 1, 0) } < 0 {
            return match std::io::Error::last_os_error().kind() {
                std::io::ErrorKind::Interrupted => continue,
                _ => false, // Can't tell; leave the client registered.
            };
        }
        return fd.revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0;
    }
}

/// Serve request frames from one client until it stops speaking.
///
/// An unparseable frame (corrupt JSON, or a payload variant this daemon does
/// not know) is skipped: the connection and its snapshot stream stay up. A
/// frame header past the cap means the stream is desynced, so reading stops —
/// the client stays registered and keeps receiving pushes.
fn serve_requests(mut reader: UnixStream, client: &Client, git: &GitRequests) {
    use super::protocol::{DaemonRequest, DaemonResponse, RequestPayload, ResponsePayload};

    while let Some(frame) = super::protocol::read_frame(&mut reader) {
        let Ok(request) = serde_json::from_slice::<DaemonRequest>(&frame) else {
            tracing::debug!("daemon: ignoring unrecognised request frame");
            continue;
        };
        let response = match request.payload {
            RequestPayload::GetGitStatuses { paths, prioritise } => {
                if prioritise && let Ok(mut queue) = git.prioritise.lock() {
                    queue.extend(paths.iter().cloned());
                }
                // Served from the live cache, never by spawning git. Unknown
                // paths are simply absent from the reply.
                let statuses = match git.cache.lock() {
                    Ok(cache) => paths
                        .into_iter()
                        .filter_map(|p| cache.get(&p).cloned().map(|s| (p, s)))
                        .collect(),
                    Err(_) => HashMap::new(),
                };
                ResponsePayload::GitStatuses(statuses)
            }
        };
        let response = DaemonResponse {
            id: request.id,
            payload: response,
        };
        let Ok(mut stream) = client.lock() else {
            return;
        };
        if super::protocol::write_frame(&mut *stream, &response).is_err() {
            return; // Client went away mid-reply; the broadcast will reap it.
        }
    }
}

/// Read the sidebar layout mode from tmux global, falling back to settings.json, then config.
fn read_sidebar_layout_mode(config: &Config, store: &StateStore) -> Option<SidebarLayoutMode> {
    // Check tmux global first (set by toggle_layout_mode during this session)
    if let Ok(output) = Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@muxix_sidebar_layout"])
        .run_and_capture_stdout()
    {
        match output.trim() {
            "tiles" => return Some(SidebarLayoutMode::Tiles),
            "compact" => return Some(SidebarLayoutMode::Compact),
            _ => {}
        }
    }

    // Fall back to persisted setting (user toggled layout in a previous tmux session)
    if let Ok(settings) = store.load_settings() {
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

/// Read pane IDs manually marked as sleeping from the tmux global option.
fn read_sleeping_panes() -> HashSet<String> {
    Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@muxix_sleeping_panes"])
        .run_and_capture_stdout()
        .ok()
        .map(|s| s.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

/// The sidebar's tmux global options, as last read.
///
/// Muxix writes all three itself and signals the daemon when it does, so
/// querying them every tick asks tmux for answers it was already pushed.
/// Re-read on a signalled tick, and once per sweep in case something outside
/// muxix set them.
struct TmuxOptions {
    read_at: Option<Instant>,
    position: SidebarPosition,
    layout_mode: SidebarLayoutMode,
    sleeping_pane_ids: HashSet<String>,
}

impl TmuxOptions {
    fn new() -> Self {
        Self {
            read_at: None,
            position: SidebarPosition::default(),
            layout_mode: SidebarLayoutMode::default(),
            sleeping_pane_ids: HashSet::new(),
        }
    }

    fn is_stale(&self, signalled: bool, now: Instant) -> bool {
        signalled
            || self
                .read_at
                .is_none_or(|at| now.saturating_duration_since(at) >= SWEEP_INTERVAL)
    }

    fn refresh(&mut self, config: &Config, store: &StateStore, now: Instant) {
        self.position = super::read_sidebar_position(config);
        self.layout_mode = read_sidebar_layout_mode(config, store).unwrap_or_default();
        self.sleeping_pane_ids = read_sleeping_panes();
        self.read_at = Some(now);
    }
}

/// Shared git status cache, updated by a background worker thread.
type GitCache = Arc<Mutex<HashMap<PathBuf, GitStatus>>>;

/// Paths a client asked to have refreshed early, drained by the git worker on
/// its next pass instead of waiting for the sweep.
type PrioritiseQueue = Arc<Mutex<HashSet<PathBuf>>>;

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

/// Build a gitignore matcher for a worktree root.
/// Loads the root .gitignore (covers the vast majority of ignored paths like
/// target/, node_modules/, .venv/, build/, etc.) without needing to walk
/// nested .gitignore files.
fn build_gitignore(worktree: &Path) -> Gitignore {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(worktree);
    if let Some(err) = builder.add(worktree.join(".gitignore")) {
        tracing::debug!(
            "failed to parse .gitignore for {}: {}",
            worktree.display(),
            err
        );
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Check if a filesystem event path should be skipped based on gitignore rules.
/// Returns true if the path is inside a .git directory (non-working-tree change)
/// or matches the worktree's .gitignore patterns.
fn is_event_ignored(
    event_path: &Path,
    worktree: &Path,
    gitignores: &HashMap<PathBuf, Gitignore>,
) -> bool {
    // Linked-worktree git metadata events (e.g. shared gitdir, common refs)
    // live outside the worktree root. They are git events, not working-tree
    // files, so they should never be ignored.
    let Ok(rel) = event_path.strip_prefix(worktree) else {
        return false;
    };

    let rel_str = rel.to_string_lossy();
    // Always process .git metadata changes (HEAD, index, refs) - they affect git status
    if rel_str.starts_with(".git/") || rel_str == ".git" {
        // Skip .git/objects and .git/logs (high volume, don't affect status)
        // but allow .git/index, .git/HEAD, .git/refs, etc.
        return rel_str.starts_with(".git/objects/") || rel_str.starts_with(".git/logs/");
    }

    if let Some(gi) = gitignores.get(worktree) {
        // Pass false for is_dir to avoid a synchronous stat syscall per event.
        // Directory-level ignore rules (e.g. "target/") still match because
        // matched_path_or_any_parents checks ancestor components.
        gi.matched_path_or_any_parents(event_path, false)
            .is_ignore()
    } else {
        false
    }
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

/// Whether the platform can handle recursive worktree watches efficiently.
/// macOS FSEvents aggregates events at the directory level in the kernel and
/// handles heavy I/O well. Linux inotify sets a watch per directory and
/// generates an event per file operation, which overwhelms the system under
/// heavy AI/MCP file activity.
fn platform_supports_worktree_watches() -> bool {
    cfg!(target_os = "macos")
}

/// Set up filesystem watches for a worktree.
///
/// Always watches .git metadata (non-recursively for HEAD/index/etc., recursively
/// for refs/) to detect commits, staging, and branch changes instantly.
///
/// On macOS (FSEvents), also watches the worktree root recursively for near-instant
/// uncommitted change detection. On Linux (inotify), working tree changes are
/// detected by the periodic poll sweep instead, avoiding the massive inotify event
/// volume that recursive watches generate under heavy file I/O.
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
            // Watch per-worktree gitdir non-recursively (HEAD, index, etc.)
            add_watch(
                watcher,
                &git_dir,
                RecursiveMode::NonRecursive,
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
    } else if dot_git.is_dir() {
        // Normal worktree: watch .git/ non-recursively (HEAD, index, MERGE_HEAD, etc.)
        add_watch(
            watcher,
            &dot_git,
            RecursiveMode::NonRecursive,
            worktree,
            watch_to_worktrees,
            &mut watched,
        );
        // Watch refs/ recursively (low volume: branch creates/deletes/updates)
        let refs_dir = dot_git.join("refs");
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
    }

    // On macOS, also watch the worktree root for near-instant uncommitted change
    // detection. FSEvents handles heavy I/O efficiently via kernel-level aggregation.
    // On Linux, skip this and rely on the periodic poll sweep instead.
    if platform_supports_worktree_watches() {
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

/// Drain the prioritise queue, keeping only paths the worker already tracks.
///
/// The cache only ever holds tracked paths, so a hint for anything else buys
/// nothing — and an unfiltered list would be an unbounded number of git spawns
/// on a client's say-so.
fn take_prioritised(queue: &PrioritiseQueue, tracked: &HashMap<PathBuf, bool>) -> Vec<PathBuf> {
    let Ok(mut queue) = queue.lock() else {
        return Vec::new();
    };
    if queue.is_empty() {
        return Vec::new();
    }
    std::mem::take(&mut *queue)
        .into_iter()
        .filter(|p| tracked.contains_key(p))
        .collect()
}

/// Refresh git status for a worktree path, updating the cache.
/// Returns true if the status actually changed (semantically, ignoring cached_at).
fn refresh_git_status(path: &Path, main_branch: Option<&str>, cache: &GitCache) -> bool {
    let prev = cache.lock().ok().and_then(|c| c.get(path).cloned());
    let new_status = crate::git::get_git_status_cached(path, main_branch, prev.as_ref());
    let changed = prev.is_none_or(|old| !git_status_semantically_equal(&old, &new_status));
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
    /// The `main_branch` governing this path, resolved by the main loop: one
    /// daemon serves many projects, so the worker has no config of its own.
    main_branch: Option<String>,
}

/// Info about an active agent path sent to the PR worker.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct PrWorkerPath {
    path: PathBuf,
    branch: String,
}

type PrPathCache = Arc<Mutex<HashMap<PathBuf, PrPathEntry>>>;
type PrRepoCache = Arc<Mutex<HashMap<PathBuf, HashMap<String, PrSummary>>>>;

fn publish_pr_path_cache(
    entries: &[PrWorkerPath],
    repo_roots: &HashMap<PathBuf, PathBuf>,
    repo_cache: &HashMap<PathBuf, HashMap<String, PrSummary>>,
    path_cache: &PrPathCache,
    dirty_flag: &Arc<AtomicBool>,
    wake_tx: &std::sync::mpsc::SyncSender<()>,
) {
    let mut next = HashMap::new();
    for entry in entries {
        if let Some(repo_root) = repo_roots.get(&entry.path)
            && let Some(pr) = repo_cache
                .get(repo_root)
                .and_then(|prs| prs.get(&entry.branch))
        {
            next.insert(
                entry.path.clone(),
                PrPathEntry {
                    branch: entry.branch.clone(),
                    summary: pr.clone(),
                },
            );
        }
    }
    let changed = if let Ok(mut cache) = path_cache.lock() {
        if *cache == next {
            false
        } else {
            *cache = next;
            true
        }
    } else {
        false
    };
    if changed {
        dirty_flag.store(true, Ordering::Relaxed);
        let _ = wake_tx.try_send(());
    }
}

fn spawn_pr_worker(
    term: Arc<AtomicBool>,
    dirty_flag: Arc<AtomicBool>,
    wake_tx: std::sync::mpsc::SyncSender<()>,
) -> (PrPathCache, std::sync::mpsc::Sender<Vec<PrWorkerPath>>) {
    let path_cache: PrPathCache = Arc::new(Mutex::new(HashMap::new()));
    let path_cache_clone = path_cache.clone();
    let repo_cache: PrRepoCache = Arc::new(Mutex::new(crate::github::load_pr_cache()));
    let (tx, rx) = std::sync::mpsc::channel::<Vec<PrWorkerPath>>();

    thread::spawn(move || {
        let mut active_entries: Vec<PrWorkerPath> = Vec::new();
        let mut repo_roots: HashMap<PathBuf, PathBuf> = HashMap::new();
        let mut last_key: Vec<(PathBuf, String)> = Vec::new();
        let mut last_fetch = Instant::now() - Duration::from_secs(30);

        while !term.load(Ordering::Relaxed) {
            let mut paths_changed = false;
            match rx.recv_timeout(Duration::from_secs(1)) {
                Ok(entries) => {
                    active_entries = entries;
                    paths_changed = true;
                    while let Ok(entries) = rx.try_recv() {
                        active_entries = entries;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if active_entries.is_empty() {
                if paths_changed {
                    if let Ok(mut cache) = path_cache_clone.lock() {
                        cache.clear();
                    }
                    dirty_flag.store(true, Ordering::Relaxed);
                    let _ = wake_tx.try_send(());
                }
                continue;
            }

            active_entries.sort();
            active_entries.dedup();
            let key: Vec<(PathBuf, String)> = active_entries
                .iter()
                .map(|entry| (entry.path.clone(), entry.branch.clone()))
                .collect();
            let branch_set_changed = key != last_key;

            if paths_changed {
                for entry in &active_entries {
                    if !repo_roots.contains_key(&entry.path)
                        && let Ok(root) = crate::git::get_repo_root_for(&entry.path)
                    {
                        repo_roots.insert(entry.path.clone(), root);
                    }
                }
                let snapshot = repo_cache
                    .lock()
                    .ok()
                    .map(|c| c.clone())
                    .unwrap_or_default();
                publish_pr_path_cache(
                    &active_entries,
                    &repo_roots,
                    &snapshot,
                    &path_cache_clone,
                    &dirty_flag,
                    &wake_tx,
                );
            }

            if !branch_set_changed && last_fetch.elapsed() < Duration::from_secs(30) {
                continue;
            }

            let mut repo_branches: HashMap<PathBuf, Vec<String>> = HashMap::new();
            for entry in &active_entries {
                if let Some(repo_root) = repo_roots.get(&entry.path) {
                    repo_branches
                        .entry(repo_root.clone())
                        .or_default()
                        .push(entry.branch.clone());
                }
            }
            for branches in repo_branches.values_mut() {
                branches.sort();
                branches.dedup();
            }
            if repo_branches.is_empty() {
                last_key = key;
                continue;
            }

            let mut fetched = HashMap::new();
            for (repo_root, branches) in repo_branches {
                match crate::github::list_prs_for_branches(&repo_root, &branches) {
                    Ok(prs) => {
                        fetched.insert(repo_root, prs);
                    }
                    Err(e) => {
                        tracing::warn!("failed to fetch PRs for {:?}: {}", repo_root, e);
                    }
                }
            }
            if !fetched.is_empty()
                && let Ok(mut cache) = repo_cache.lock()
            {
                for (repo_root, prs) in &fetched {
                    if prs.is_empty() {
                        cache.remove(repo_root);
                    } else {
                        cache.insert(repo_root.clone(), prs.clone());
                    }
                }
                crate::github::save_pr_cache(&fetched);
                publish_pr_path_cache(
                    &active_entries,
                    &repo_roots,
                    &cache,
                    &path_cache_clone,
                    &dirty_flag,
                    &wake_tx,
                );
            }
            last_key = key;
            last_fetch = Instant::now();
        }
    });

    (path_cache, tx)
}

/// The periodic git sweep interval, derived from what the watcher can see.
fn git_sweep_interval(has_watcher: bool) -> Duration {
    if !has_watcher {
        // No watcher available: poll frequently as the only change detection.
        Duration::from_secs(2)
    } else if platform_supports_worktree_watches() {
        // macOS: worktree watches give instant detection, sweep is just a safety net.
        Duration::from_secs(30)
    } else {
        // Linux: only .git metadata is watched, so working tree changes need
        // polling. A refresh that finds nothing moved costs one git process.
        Duration::from_secs(10)
    }
}

/// Spawn a background thread that watches for git changes and updates the cache.
///
/// Uses the `notify` crate for OS-level filesystem event detection (FSEvents on macOS).
/// Watches .git internals and worktree roots for each active worktree. Events are
/// debounced per-worktree (300ms) before triggering a refresh. A fallback
/// sweep covers what the watcher cannot see; see `git_sweep_interval`.
fn spawn_git_worker(
    term: Arc<AtomicBool>,
    dirty_flag: Arc<AtomicBool>,
    wake_tx: std::sync::mpsc::SyncSender<()>,
    prioritise: PrioritiseQueue,
) -> (GitCache, std::sync::mpsc::Sender<Vec<GitWorkerPath>>) {
    let cache: GitCache = Arc::new(Mutex::new(HashMap::new()));
    let cache_clone = cache.clone();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<GitWorkerPath>>();

    thread::spawn(move || {
        // Bounded filesystem event channel to prevent unbounded memory growth
        // under heavy file I/O (e.g. MCP servers, Claude sessions).
        // On overflow, all worktrees are marked pending for an early refresh.
        let (fs_tx, fs_rx) = std::sync::mpsc::sync_channel(256);
        let fs_overflow = Arc::new(AtomicBool::new(false));
        let fs_overflow_clone = fs_overflow.clone();
        let mut watcher: Option<notify::RecommendedWatcher> = match notify::RecommendedWatcher::new(
            move |event: notify::Result<notify::Event>| {
                if let Ok(ref e) = event {
                    // Filter out .git internal traffic that doesn't affect status.
                    // Gitignore-based filtering (node_modules, target, etc.) happens
                    // in the worker thread where matchers are available.
                    let dominated_by_noise = e.paths.iter().all(|p| {
                        let s = p.to_string_lossy();
                        s.contains("/.git/objects/") || s.contains("/.git/logs/")
                    });
                    if dominated_by_noise {
                        return;
                    }
                }
                if let Err(std::sync::mpsc::TrySendError::Full(_)) = fs_tx.try_send(event) {
                    fs_overflow_clone.store(true, Ordering::Relaxed);
                }
            },
            notify::Config::default(),
        ) {
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
        // Per-worktree gitignore matchers for filtering irrelevant events
        let mut gitignores: HashMap<PathBuf, Gitignore> = HashMap::new();
        // Per-worktree: timestamp of last fs event (for debouncing)
        let mut pending_worktrees: HashMap<PathBuf, Instant> = HashMap::new();
        // Stale status per path (true = all agents at path are stale)
        let mut path_stale: HashMap<PathBuf, bool> = HashMap::new();
        // Configured main_branch per path, as resolved by the main loop
        let mut path_main: HashMap<PathBuf, Option<String>> = HashMap::new();
        // Track unique active paths for fallback polling
        let mut unique_active: Vec<PathBuf> = Vec::new();
        let mut last_full_sweep = Instant::now();
        let debounce_duration = Duration::from_millis(300);

        while !term.load(Ordering::Relaxed) {
            let full_sweep_interval = git_sweep_interval(watcher.is_some());
            // Block on filesystem events (zero CPU when idle), or sleep briefly in poll mode
            if watcher.is_some() {
                let timeout = next_worker_timeout(
                    &pending_worktrees,
                    debounce_duration,
                    last_full_sweep,
                    full_sweep_interval,
                );
                // Process a single FS event: find affected worktrees,
                // skip gitignored paths, and mark worktrees as pending.
                let mut process_event = |event: notify::Event| {
                    for path in &event.paths {
                        // Rebuild gitignore matcher when .gitignore itself changes
                        if path.file_name().is_some_and(|n| n == ".gitignore") {
                            for wt in find_worktrees_for_path(path, &watch_to_worktrees) {
                                let gi = build_gitignore(&wt);
                                gitignores.insert(wt, gi);
                            }
                        }
                        for wt in find_worktrees_for_path(path, &watch_to_worktrees) {
                            if is_event_ignored(path, &wt, &gitignores) {
                                continue;
                            }
                            pending_worktrees.entry(wt).or_insert_with(Instant::now);
                        }
                    }
                };

                match fs_rx.recv_timeout(timeout) {
                    Ok(Ok(event)) => process_event(event),
                    Ok(Err(e)) => {
                        tracing::warn!("filesystem watch error: {}", e);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }

                // Drain any additional buffered events
                while let Ok(event_result) = fs_rx.try_recv() {
                    if let Ok(event) = event_result {
                        process_event(event);
                    }
                }

                // On channel overflow, mark all active worktrees as pending so
                // events for one noisy worktree can't starve updates to others.
                if fs_overflow.swap(false, Ordering::Relaxed) {
                    let now = Instant::now();
                    for path in worktree_watches.keys() {
                        pending_worktrees.entry(path.clone()).or_insert(now);
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
                path_main.clear();
                for entry in &active_entries {
                    let e = path_stale.entry(entry.path.clone()).or_insert(true);
                    if !entry.is_stale {
                        *e = false;
                    }
                    path_main.insert(entry.path.clone(), entry.main_branch.clone());
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
                        gitignores.remove(path);
                        pending_worktrees.remove(path);
                    }

                    // Add watches for new worktrees
                    for path in &unique_active {
                        if worktree_watches.contains_key(path) {
                            continue;
                        }
                        let watched = setup_worktree_watches(w, path, &mut watch_to_worktrees);
                        worktree_watches.insert(path.clone(), watched);
                        gitignores.insert(path.clone(), build_gitignore(path));
                        // Trigger immediate status fetch for new worktrees
                        pending_worktrees.insert(path.clone(), Instant::now() - debounce_duration);
                    }

                    // Prune cache for removed worktrees
                    if !removed.is_empty() {
                        if let Ok(mut c) = cache_clone.lock() {
                            c.retain(|p, _| unique_set.contains(p));
                        }
                        dirty_flag.store(true, Ordering::Relaxed);
                        let _ = wake_tx.try_send(());
                    }
                } else {
                    // Poll-only mode: just prune cache, no watches to manage
                    if let Ok(mut c) = cache_clone.lock() {
                        let before = c.len();
                        c.retain(|p, _| unique_set.contains(p));
                        if c.len() != before {
                            dirty_flag.store(true, Ordering::Relaxed);
                            let _ = wake_tx.try_send(());
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

            let mut any_changed = false;

            // A client asked for these by name, so refresh them now — past the
            // debounce, and past the stale-agent skip.
            let hinted = take_prioritised(&prioritise, &path_stale);
            for path in &hinted {
                pending_worktrees.remove(path);
                if refresh_git_status(
                    path,
                    path_main.get(path).and_then(|b| b.as_deref()),
                    &cache_clone,
                ) {
                    any_changed = true;
                }
            }

            // Process debounce-ready worktrees (skip stale ones, they only refresh on sweep)
            let now = Instant::now();
            let ready: Vec<PathBuf> = pending_worktrees
                .iter()
                .filter(|(_, last_event)| now.duration_since(**last_event) >= debounce_duration)
                .map(|(path, _)| path.clone())
                .collect();

            for path in &ready {
                pending_worktrees.remove(path);
                let is_stale = path_stale.get(path).copied().unwrap_or(false);
                if is_stale {
                    continue;
                }
                if refresh_git_status(
                    path,
                    path_main.get(path).and_then(|b| b.as_deref()),
                    &cache_clone,
                ) {
                    any_changed = true;
                }
            }

            // Fallback full sweep (see `git_sweep_interval`; includes stale worktrees)
            if last_full_sweep.elapsed() >= full_sweep_interval {
                last_full_sweep = Instant::now();
                let sweep_paths: Vec<PathBuf> = if watcher.is_some() {
                    worktree_watches.keys().cloned().collect()
                } else {
                    unique_active.clone()
                };
                for path in &sweep_paths {
                    pending_worktrees.remove(path);
                    if refresh_git_status(
                        path,
                        path_main.get(path).and_then(|b| b.as_deref()),
                        &cache_clone,
                    ) {
                        any_changed = true;
                    }
                }
            }

            if any_changed {
                dirty_flag.store(true, Ordering::Relaxed);
                let _ = wake_tx.try_send(());
            }
        }
    });

    (cache, tx)
}

/// Spawn a thread that watches the global config file and per-project
/// `.muxix.yaml` files and bumps `config_version` whenever a reload succeeds.
///
/// Returns a channel for the daemon main loop to send the current set of
/// project config directories (parents of `.muxix.yaml`) to watch.
fn spawn_config_watcher(
    term: Arc<AtomicBool>,
    config: Arc<Mutex<Config>>,
    config_version: Arc<AtomicU64>,
    dirty_flag: Arc<AtomicBool>,
    wake_tx: mpsc::SyncSender<()>,
) -> mpsc::Sender<HashSet<PathBuf>> {
    let (paths_tx, paths_rx) = mpsc::channel::<HashSet<PathBuf>>();
    thread::spawn(move || {
        // Bounded fs event channel; on overflow force a reload.
        let (fs_tx, fs_rx) = mpsc::sync_channel::<notify::Result<notify::Event>>(64);
        let overflow = Arc::new(AtomicBool::new(false));
        let overflow_clone = overflow.clone();
        let mut watcher: notify::RecommendedWatcher = match notify::RecommendedWatcher::new(
            move |event: notify::Result<notify::Event>| {
                if let Err(mpsc::TrySendError::Full(_)) = fs_tx.try_send(event) {
                    overflow_clone.store(true, Ordering::Relaxed);
                }
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!("config watcher unavailable: {}", e);
                return;
            }
        };

        // Track watched directories so we can reconcile add/remove and avoid
        // re-watching the same path twice.
        let mut watched_global: Option<PathBuf> = None;
        let mut watched_project_dirs: HashSet<PathBuf> = HashSet::new();
        let mut pending_reload_at: Option<Instant> = None;
        let debounce = Duration::from_millis(200);

        // Watch the global config dir non-recursively. Watching the parent dir
        // (rather than the file) catches atomic-rename saves: write to a
        // sibling temp file, then rename(temp, target). This is what vim,
        // claude-code's Edit/Write tools, and most editors do. A direct file
        // watch would lose the inode on rename and miss subsequent edits. It
        // also fires on first-time creation when no config exists yet.
        if let Some(p) = crate::config::global_config_path()
            && let Some(dir) = p.parent()
        {
            match watcher.watch(dir, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    tracing::info!(
                        op = "watch",
                        path = %dir.display(),
                        kind = "global",
                        "fd-leak debug (config)"
                    );
                    watched_global = Some(dir.to_path_buf());
                }
                Err(e) => {
                    tracing::warn!("failed to watch global config dir {}: {}", dir.display(), e);
                }
            }
        }

        let interesting_basenames = ["config.yaml", "config.yml", ".muxix.yaml", ".muxix.yml"];

        while !term.load(Ordering::Relaxed) {
            // 1. Reconcile per-project watches from incoming path sets.
            while let Ok(new_dirs) = paths_rx.try_recv() {
                let to_remove: Vec<PathBuf> = watched_project_dirs
                    .difference(&new_dirs)
                    .cloned()
                    .collect();
                for dir in &to_remove {
                    // Never unwatch the global config dir, even if it was
                    // tracked under watched_project_dirs (we never issued an
                    // OS-level watch for it from the project path; it's still
                    // watched as the global watch).
                    if Some(dir) == watched_global.as_ref() {
                        watched_project_dirs.remove(dir);
                        continue;
                    }
                    let res = watcher.unwatch(dir);
                    tracing::info!(
                        op = "unwatch",
                        path = %dir.display(),
                        ok = res.is_ok(),
                        kind = "project",
                        total = watched_project_dirs.len() - 1,
                        "fd-leak debug (config)"
                    );
                    watched_project_dirs.remove(dir);
                }
                let to_add: Vec<PathBuf> = new_dirs
                    .difference(&watched_project_dirs)
                    .cloned()
                    .collect();
                for dir in to_add {
                    // Skip if it's the same as the global watched dir to avoid
                    // double-watching the same path.
                    if Some(&dir) == watched_global.as_ref() {
                        watched_project_dirs.insert(dir);
                        continue;
                    }
                    match watcher.watch(&dir, RecursiveMode::NonRecursive) {
                        Ok(()) => {
                            tracing::info!(
                                op = "watch",
                                path = %dir.display(),
                                kind = "project",
                                total = watched_project_dirs.len() + 1,
                                "fd-leak debug (config)"
                            );
                            watched_project_dirs.insert(dir);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "failed to watch project config dir {}: {}",
                                dir.display(),
                                e
                            );
                        }
                    }
                }
            }

            // 2. Wait for the next event, capped by the pending debounce deadline.
            let timeout = pending_reload_at
                .map(|t| t.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|| Duration::from_millis(500));

            match fs_rx.recv_timeout(timeout) {
                Ok(Ok(event)) => {
                    let interesting = event.paths.iter().any(|p| {
                        p.file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| interesting_basenames.contains(&n))
                    });
                    if interesting {
                        // Lock the deadline on the FIRST event in a burst; do
                        // not slide it forward on every subsequent event.
                        pending_reload_at.get_or_insert(Instant::now() + debounce);
                    }
                }
                Ok(Err(e)) => tracing::warn!("config watch error: {}", e),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            if overflow.swap(false, Ordering::Relaxed) {
                pending_reload_at.get_or_insert(Instant::now() + debounce);
            }

            // 3. Reload if the debounce deadline has passed.
            if let Some(t) = pending_reload_at
                && Instant::now() >= t
            {
                pending_reload_at = None;
                // Always bump the version so clients try their own per-project
                // load (their anchor path may differ from the daemon CWD; a
                // failure here doesn't necessarily mean clients will fail).
                // Only update the daemon-side cached Config on success.
                match Config::load(None) {
                    Ok(new_cfg) => {
                        if let Ok(mut slot) = config.lock() {
                            *slot = new_cfg;
                        }
                        tracing::debug!("daemon config reloaded");
                    }
                    Err(e) => {
                        tracing::warn!("daemon-side config load failed, keeping previous: {}", e);
                    }
                }
                let v = config_version.fetch_add(1, Ordering::Relaxed) + 1;
                tracing::info!(version = v, "sidebar config_version bumped");
                dirty_flag.store(true, Ordering::Relaxed);
                let _ = wake_tx.try_send(());
            }
        }
    });

    paths_tx
}

/// Detects working agents that have stopped producing output.
///
/// # Behavior
/// - A working agent with no pane output and no RPC activity for >= timeout
///   is considered interrupted.
/// - Interrupted state is sticky: only an RPC update from the agent clears
///   it. User typing or cursor movement in the pane does not.
/// - After clearing, the agent gets a fresh timeout window before it can
///   be marked interrupted again.
/// - Interrupted agents show no icon and no timer in the sidebar.
/// - When an agent resumes, the timer resets to zero.
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

    /// Whether this pane is confirmed interrupted and capture can be skipped.
    fn is_confirmed(&self, pane_id: &str, updated_ts: u64) -> bool {
        self.confirmed
            .get(pane_id)
            .is_some_and(|&ts| updated_ts <= ts)
    }

    /// Check all working agents for inactivity. Returns the set of pane IDs
    /// that appear interrupted (content unchanged for longer than timeout).
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
            if let Some(confirmed_ts) = self.confirmed.remove(id) {
                let updated_ts = working
                    .get(id.as_str())
                    .and_then(|a| a.updated_ts)
                    .unwrap_or(0);
                tracing::info!(
                    pane_id = %id,
                    confirmed_ts,
                    updated_ts,
                    "agent inactivity cleared"
                );
            }
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
                    let idle_for = now.duration_since(first_seen);
                    if idle_for >= self.timeout
                        && self
                            .confirmed
                            .insert(pane_id.to_string(), current_rpc)
                            .is_none()
                    {
                        tracing::info!(
                            pane_id = %pane_id,
                            updated_ts = current_rpc,
                            idle_for_ms = idle_for.as_millis(),
                            timeout_ms = self.timeout.as_millis(),
                            "agent inactivity detected"
                        );
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

/// Walk the process tree rooted at `root_pid` via `/proc/<pid>/task/<tid>/children`,
/// returning all descendant PIDs (not including `root_pid` itself).
#[cfg(target_os = "linux")]
fn proc_descendants(root_pid: u32) -> Vec<u32> {
    let mut result = Vec::new();
    let mut queue = vec![root_pid];
    while let Some(pid) = queue.pop() {
        let children_path = format!("/proc/{pid}/task/{pid}/children");
        let Ok(contents) = std::fs::read_to_string(&children_path) else {
            continue;
        };
        for tok in contents.split_whitespace() {
            let Ok(child) = tok.parse::<u32>() else {
                continue;
            };
            result.push(child);
            queue.push(child);
        }
    }
    result
}

#[cfg(not(target_os = "linux"))]
fn proc_descendants(_root_pid: u32) -> Vec<u32> {
    Vec::new()
}

fn sigstop_pids(pids: &[u32]) {
    for &pid in pids {
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGSTOP) };
    }
}

fn sigcont_pids(pids: &[u32]) {
    for &pid in pids {
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGCONT) };
    }
}

const FREEZE_PREFIX: &str = "❄ ";

fn rename_window_by_id(window_id: &str, name: &str) {
    let _ = Cmd::new("tmux")
        .args(&["rename-window", "-t", window_id, name])
        .run();
}

struct FrozenEntry {
    child_pids: Vec<u32>,
    /// Stable tmux window ID (e.g. "@42") — used for rename, survives window renames.
    window_id: String,
    /// Original window name before the freeze prefix was applied.
    original_window_name: String,
}

/// Tracks agent panes that have been SIGSTOPed and their frozen metadata.
/// On each daemon tick it freezes newly-idle Done agents and thaws any that
/// have regained pane focus.
struct FreezeTracker {
    /// pane_id → frozen state
    frozen: HashMap<String, FrozenEntry>,
}

impl FreezeTracker {
    fn new() -> Self {
        Self {
            frozen: HashMap::new(),
        }
    }

    /// Thaw all frozen agents — called on daemon shutdown.
    fn thaw_all(&mut self) {
        for (pane_id, entry) in self.frozen.drain() {
            tracing::info!(pane_id, "thawing agent (daemon shutdown)");
            sigcont_pids(&entry.child_pids);
            rename_window_by_id(&entry.window_id, &entry.original_window_name);
        }
    }

    /// Called once per daemon tick.
    ///
    /// - Thaws agents whose pane is now active.
    /// - Thaws agents that resumed working (status changed away from Done).
    /// - Freezes Done agents that have been unfocused longer than `after_idle`.
    fn tick(
        &mut self,
        agents: &[crate::multiplexer::AgentPane],
        active_pane_ids: &HashSet<String>,
        after_idle: std::time::Duration,
        now_ts: u64,
    ) {
        use crate::multiplexer::AgentStatus;

        // Build a quick lookup of live pane ids
        let live_ids: HashSet<&str> = agents.iter().map(|a| a.pane_id.as_str()).collect();

        // Thaw: pane is focused, status is no longer Done, or pane disappeared
        let to_thaw: Vec<String> = self
            .frozen
            .keys()
            .filter(|id| {
                active_pane_ids.contains(*id)
                    || !live_ids.contains(id.as_str())
                    || agents
                        .iter()
                        .find(|a| &a.pane_id == *id)
                        .is_some_and(|a| a.status != Some(AgentStatus::Done))
            })
            .cloned()
            .collect();
        for pane_id in to_thaw {
            if let Some(entry) = self.frozen.remove(&pane_id) {
                tracing::info!(pane_id, "thawing agent");
                sigcont_pids(&entry.child_pids);
                rename_window_by_id(&entry.window_id, &entry.original_window_name);
            }
        }

        // Freeze: Done, unfocused, idle longer than threshold, not already frozen
        let after_idle_secs = after_idle.as_secs();
        for agent in agents {
            if self.frozen.contains_key(&agent.pane_id) {
                continue;
            }
            if active_pane_ids.contains(&agent.pane_id) {
                continue;
            }
            if agent.status != Some(AgentStatus::Done) {
                continue;
            }
            if agent.pane_pid == 0 || agent.window_id.is_empty() {
                continue;
            }
            let idle_secs = agent
                .status_ts
                .map(|ts| now_ts.saturating_sub(ts))
                .unwrap_or(0);
            if idle_secs < after_idle_secs {
                continue;
            }
            let children = proc_descendants(agent.pane_pid);
            if children.is_empty() {
                continue;
            }
            tracing::info!(
                pane_id = %agent.pane_id,
                pane_pid = agent.pane_pid,
                idle_secs,
                children = children.len(),
                window_id = %agent.window_id,
                "freezing idle agent"
            );
            sigstop_pids(&children);
            let frozen_name = format!("{}{}", FREEZE_PREFIX, agent.window_name);
            rename_window_by_id(&agent.window_id, &frozen_name);
            self.frozen.insert(
                agent.pane_id.clone(),
                FrozenEntry {
                    child_pids: children,
                    window_id: agent.window_id.clone(),
                    original_window_name: agent.window_name.clone(),
                },
            );
        }
    }
}

/// Run the muxix daemon (headless, no TUI).
pub fn run() -> Result<()> {
    let mux = create_backend(detect_backend());
    let instance_id = mux.instance_id();
    let config = Arc::new(Mutex::new(Config::load(None)?));
    // Captured at startup and intentionally not live-reloaded. tmux's
    // @muxix_pane_status holds the icon string itself; build_snapshot
    // compares pane statuses to these exact strings to suppress stale
    // done/waiting markers, so swapping the icons mid-run would mis-suppress.
    let status_icons = config.lock().unwrap().status_icons.clone();
    let config_version = Arc::new(AtomicU64::new(0));

    tracing::info!(instance_id = %instance_id, "muxix daemon starting");

    // Signal handlers for clean shutdown and dirty notification
    let term = Arc::new(AtomicBool::new(false));
    let dirty_flag = Arc::new(AtomicBool::new(false));
    // SIGUSR2: the ADE listing changed (an agent was deleted or started here),
    // so the next tick must not serve the cached one.
    let ade_dirty = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, term.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGUSR1, dirty_flag.clone())?;
    signal_hook::flag::register(signal_hook::consts::SIGUSR2, ade_dirty.clone())?;

    // Wake channel: replaces the 10ms spin loop. Producers send () to wake the
    // main loop immediately (SIGUSR1 uses the AtomicBool since signal handlers
    // can't send on channels; the wake channel handles git worker notifications).
    let (wake_tx, wake_rx) = std::sync::mpsc::sync_channel::<()>(1);
    // Keep a sender alive so recv_timeout won't return Disconnected if the
    // git worker thread panics (which would spin the main loop at 100% CPU).
    let _wake_tx_keepalive = wake_tx.clone();

    // The handlers above only set flags, which the main loop would have to
    // poll. This thread turns the same signals into a wake, so the loop can
    // block on the channel until there is real work.
    {
        let mut signals = signal_hook::iterator::Signals::new([
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGUSR1,
            signal_hook::consts::SIGUSR2,
        ])?;
        let wake_on_signal = wake_tx.clone();
        thread::spawn(move || {
            for _ in signals.forever() {
                let _ = wake_on_signal.try_send(());
            }
        });
    }

    // Config watcher: bumps config_version on global / project .muxix.yaml changes.
    let config_paths_tx = spawn_config_watcher(
        term.clone(),
        config.clone(),
        config_version.clone(),
        dirty_flag.clone(),
        wake_tx.clone(),
    );

    // Background git status worker (shares dirty_flag for immediate broadcast on changes)
    let prioritise: PrioritiseQueue = Arc::new(Mutex::new(HashSet::new()));
    let (git_cache, git_path_tx) = spawn_git_worker(
        term.clone(),
        dirty_flag.clone(),
        wake_tx.clone(),
        prioritise.clone(),
    );

    // Bound after the git worker: the accept thread answers status requests
    // from its cache.
    let sock_path = socket_path(&instance_id);
    let _ = std::fs::remove_file(&sock_path); // Clean stale
    let server = SocketServer::bind(
        &sock_path,
        GitRequests {
            cache: git_cache.clone(),
            prioritise,
        },
    )?;
    let (pr_cache, pr_path_tx) = spawn_pr_worker(term.clone(), dirty_flag.clone(), wake_tx);

    // Store PID so toggle-off can kill us and hooks can signal us
    Cmd::new("tmux")
        .args(&[
            "set-option",
            "-g",
            "@muxix_daemon_pid",
            &std::process::id().to_string(),
        ])
        .run()?;

    let mut inactivity_tracker = InactivityTracker::new(INACTIVITY_TIMEOUT);
    let mut last_interrupted: HashSet<String> = HashSet::new();
    let mut freeze_tracker = FreezeTracker::new();
    let auto_freeze_duration = config
        .lock()
        .unwrap()
        .daemon
        .auto_freeze
        .as_ref()
        .map(|c| c.after_idle.as_duration());
    let mut last_runtime_write = Instant::now();
    let backend_name = mux.name().to_string();

    let mut last_refresh = Instant::now() - SWEEP_INTERVAL; // force an immediate first tick
    let mut last_client_seen = Instant::now();
    let mut dirty_pending = false;
    let mut last_agent_list = String::new();
    let mut last_health_log = Instant::now();
    let mut last_sync: std::collections::BTreeMap<String, Instant> = Default::default();
    let mut jobs_configured = config.lock().unwrap().background_jobs_configured();
    // Refresh is event-driven: agent status hooks and tmux lifecycle hooks
    // SIGUSR1 us. The timer only backstops stall detection (5s while agents
    // are working, since silence produces no events) and lost hooks (30s).
    let mut any_working = true;
    let debounce_interval = Duration::from_secs(1);

    // Cache of agent_path -> governing project config so we don't run the
    // walk-up filesystem search (plus a `git rev-parse`) and a YAML read on
    // every tick. A miss is cached too, as `None`: an agent in a tree with no
    // `.muxix.yaml` is the common case and paid for the search every tick.
    // Misses expire on the sweep, because a `.muxix.yaml` can appear in a
    // directory the config watcher was never told to watch.
    let mut project_configs =
        ProjectConfigs::new(config_version.load(Ordering::Relaxed), Instant::now());
    let mut last_config_dirs: HashSet<PathBuf> = HashSet::new();
    // ADE listings shell out, so they run far less often than the tick.
    let mut ade = AdeAgents::new();
    let mut tmux_options = TmuxOptions::new();

    while !term.load(Ordering::Relaxed) {
        // Coalesce dirty signals: SIGUSR1 sets the flag, we service it once
        // per debounce interval to prevent signal floods from causing CPU storms
        if dirty_flag.swap(false, Ordering::Relaxed) {
            dirty_pending = true;
        }
        if ade_dirty.swap(false, Ordering::Relaxed) {
            ade.polled = None;
            dirty_pending = true;
        }

        // Working agents need periodic sampling (stall detection watches for
        // *absence* of output, which no hook can signal); otherwise the timer
        // is just drift insurance against lost hook events.
        let refresh_interval = if any_working {
            WORKING_TICK_INTERVAL
        } else {
            SWEEP_INTERVAL
        };
        let time_since_refresh = last_refresh.elapsed();
        let debounce_cleared = dirty_pending && time_since_refresh >= debounce_interval;
        let timer_expired = time_since_refresh >= refresh_interval;

        if debounce_cleared || timer_expired {
            // Whether a producer asked for this tick, as opposed to the timer:
            // the cue to re-read state only muxix itself writes.
            let signalled = dirty_pending;
            dirty_pending = false;
            last_refresh = Instant::now();

            // ── Gather inputs ──
            let tmux_state = query_tmux_state();
            if tmux_state.server_gone {
                tracing::info!("muxix daemon exiting: tmux server gone");
                break;
            }
            let Ok(store) = StateStore::new() else {
                continue;
            };
            // Cloned out of the lock: sync shells out to the ADE and must not
            // hold up the config watcher's reload meanwhile.
            let cfg = config.lock().unwrap().clone();
            // Background jobs ride the tick whether or not a sidebar is open.
            let ades = crate::projects::sync::effective_ades(&cfg);
            crate::projects::sync::sync_all_quiet(&ades, &mut last_sync);
            jobs_configured = crate::projects::sync::any_sync_on(&ades);
            let active_pane_ids = tmux_state.active_pane_ids.clone();
            let Ok(mut agents) = store.load_reconciled_agents(mux.as_ref()) else {
                continue;
            };
            // Agents an ADE owns have no pane, so reconciliation never sees
            // them. Poll the runtimes for those and fold them in, so the sidebar
            // shows the work you started from your phone next to the work in
            // your panes.
            agents.extend(ade.poll(&cfg));
            // A session taken over locally shows the takeover's status, not the
            // manager's `idle`, and `exited` once it is quit.
            crate::command::ade_view::apply_takeovers(
                mux.as_ref(),
                &mut agents,
                ade.last_problem.is_none(),
                &cfg,
            );

            if tmux_options.is_stale(signalled, last_refresh) {
                tmux_options.refresh(&cfg, &store, last_refresh);
            }
            let position = tmux_options.position;
            let layout_mode = tmux_options.layout_mode;
            let sleeping_pane_ids = tmux_options.sleeping_pane_ids.clone();
            let git_statuses = git_cache.lock().ok().map(|c| c.clone()).unwrap_or_default();
            let pr_statuses = pr_cache.lock().ok().map(|c| c.clone()).unwrap_or_default();
            let now = Instant::now();
            let now_ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let captured_panes =
                gather_captures(&agents, mux.as_ref(), &inactivity_tracker, now_ts);
            let heartbeat_due = last_runtime_write.elapsed() >= Duration::from_secs(10);

            // ── Compute tick (no I/O) ──
            let mut output = compute_tick(
                TickInput {
                    agents,
                    tmux_state,
                    captured_panes,
                    now,
                    now_ts,
                    position,
                    layout_mode,
                    git_statuses,
                    pr_statuses,
                    sleeping_pane_ids,
                },
                &mut inactivity_tracker,
                &last_interrupted,
                &status_icons,
                heartbeat_due,
            );

            // ── Apply side effects, then commit state ──
            if apply_tick_effects(&output, &store, &backend_name, &instance_id) {
                last_runtime_write = Instant::now();
            }
            last_interrupted = output.next_interrupted;

            // ── Auto-freeze idle agents ──
            if let Some(after_idle) = auto_freeze_duration {
                freeze_tracker.tick(
                    &output.snapshot.agents,
                    &active_pane_ids,
                    after_idle,
                    now_ts,
                );
            }

            // ── Stamp config version + broadcast ──
            output.snapshot.config_version = config_version.load(Ordering::Relaxed);
            server.broadcast(&output.snapshot);

            any_working = output
                .snapshot
                .agents
                .iter()
                .any(|a| a.status == Some(AgentStatus::Working));

            // Resolve the project config governing each agent path once: the
            // config watcher needs the dirs, the git worker needs each path's
            // main_branch.
            let agent_paths: Vec<PathBuf> = output
                .snapshot
                .agents
                .iter()
                .map(|a| a.path.clone())
                .collect();
            let governing = project_configs.resolve_all(
                &agent_paths,
                config_version.load(Ordering::Relaxed),
                Instant::now(),
                &governing_config_of,
            );

            // Update git worker with current agent paths, stale status and base
            let stale_threshold = 60 * 60; // 1 hour, matches sidebar UI
            let entries: Vec<GitWorkerPath> = output
                .snapshot
                .agents
                .iter()
                .map(|a| GitWorkerPath {
                    path: a.path.clone(),
                    is_stale: a
                        .status_ts
                        .map(|ts| now_ts.saturating_sub(ts) > stale_threshold)
                        .unwrap_or(false),
                    main_branch: main_branch_for(governing, &a.path, cfg.main_branch.as_deref()),
                })
                .collect();
            let _ = git_path_tx.send(entries);

            let pr_entries: Vec<PrWorkerPath> = output
                .snapshot
                .agents
                .iter()
                .filter_map(|a| {
                    let branch = output.snapshot.git_statuses.get(&a.path)?.branch.as_ref()?;
                    if branch == "main" || branch == "master" {
                        return None;
                    }
                    Some(PrWorkerPath {
                        path: a.path.clone(),
                        branch: branch.clone(),
                    })
                })
                .collect();
            let _ = pr_path_tx.send(pr_entries);

            // Update config watcher with current project-config dirs.
            let config_dirs = config_dirs_of(governing);
            if config_dirs != last_config_dirs {
                let _ = config_paths_tx.send(config_dirs.clone());
                last_config_dirs = config_dirs;
            }

            let agent_list: String = output
                .snapshot
                .agents
                .iter()
                .map(|a| a.pane_id.as_str())
                .collect::<Vec<_>>()
                .join(" ");

            if agent_list != last_agent_list {
                if !agent_list.is_empty() {
                    let _ = Cmd::new("tmux")
                        .args(&["set-option", "-g", "@muxix_sidebar_agents", &agent_list])
                        .run();
                } else {
                    let _ = Cmd::new("tmux")
                        .args(&["set-option", "-gu", "@muxix_sidebar_agents"])
                        .run();
                }
                last_agent_list = agent_list;
            }
        }

        // Track client activity for auto-exit. With a job configured the daemon
        // is the job's host, so losing the last sidebar client is not an exit.
        let cc = server.client_count();
        if cc > 0 || jobs_configured {
            last_client_seen = Instant::now();
        } else if last_client_seen.elapsed() > IDLE_EXIT_TIMEOUT {
            tracing::info!("muxix daemon exiting: no clients for 10s");
            break;
        }

        // Periodic health log (every 60s).
        let health_interval = Duration::from_secs(60);
        if last_health_log.elapsed() >= health_interval {
            tracing::info!(clients = cc, jobs = jobs_configured, "muxix daemon alive");
            last_health_log = Instant::now();
        }

        // Block until woken or the soonest real deadline. Signals reach the
        // wake channel through the forwarding thread, so no flag is polled.
        let idle_exit_in = (cc == 0 && !jobs_configured)
            .then(|| IDLE_EXIT_TIMEOUT.saturating_sub(last_client_seen.elapsed()));
        let wait = next_loop_wait(
            last_refresh.elapsed(),
            if dirty_pending {
                debounce_interval
            } else {
                refresh_interval
            },
            idle_exit_in,
            health_interval.saturating_sub(last_health_log.elapsed()),
        );
        let _ = wake_rx.recv_timeout(wait);
    }

    if term.load(Ordering::Relaxed) {
        tracing::info!("muxix daemon exiting: SIGTERM received");
    }

    // Thaw any frozen agents so they don't stay SIGSTOPed after daemon exit.
    freeze_tracker.thaw_all();

    // Cleanup
    let _ = std::fs::remove_file(&sock_path);
    if let Ok(store) = StateStore::new() {
        store.delete_runtime(&backend_name, &instance_id);
    }
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-gu", "@muxix_daemon_pid"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-gu", "@muxix_sidebar_agents"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-gu", "@muxix_sleeping_panes"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-gu", "@muxix_sidebar_scope"])
        .run();
    Ok(())
}

// ── Tick core ────────────────────────────────────────────────────────────

/// Inputs gathered from the environment for one daemon tick.
struct TickInput {
    agents: Vec<crate::multiplexer::AgentPane>,
    tmux_state: TmuxState,
    captured_panes: HashMap<String, String>,
    now: Instant,
    now_ts: u64,
    position: SidebarPosition,
    layout_mode: SidebarLayoutMode,
    git_statuses: HashMap<PathBuf, GitStatus>,
    pr_statuses: HashMap<PathBuf, PrPathEntry>,
    sleeping_pane_ids: HashSet<String>,
}

/// A state-file write to apply after computing the tick.
struct AgentWrite {
    pane_id: String,
    status_ts: u64,
}

/// Output of a single tick computation.
struct TickOutput {
    snapshot: super::snapshot::SidebarSnapshot,
    agent_writes: Vec<AgentWrite>,
    runtime_write: Option<crate::state::RuntimeState>,
    /// The new interrupted set. Caller should commit to `last_interrupted`
    /// only after side effects are applied successfully.
    next_interrupted: HashSet<String>,
}

/// Compute one daemon tick from in-memory inputs.
///
/// 1. Runs inactivity detection
/// 2. Mutates agents in memory (status_ts reset for resumed agents)
/// 3. Builds the snapshot from the already-mutated agents
/// 4. Returns side effects (state file writes, runtime file write)
#[allow(clippy::too_many_arguments)]
fn compute_tick(
    input: TickInput,
    tracker: &mut InactivityTracker,
    last_interrupted: &HashSet<String>,
    status_icons: &crate::config::StatusIcons,
    heartbeat_due: bool,
) -> TickOutput {
    let TickInput {
        mut agents,
        tmux_state,
        captured_panes,
        now,
        now_ts,
        position,
        layout_mode,
        git_statuses,
        pr_statuses,
        sleeping_pane_ids,
    } = input;

    // Phase 1: Inactivity detection
    let interrupted =
        tracker.check_with(&agents, now, |pane_id| captured_panes.get(pane_id).cloned());

    // Phase 2: Mutate agents in memory for resumed agents
    let mut agent_writes = Vec::new();
    if !last_interrupted.is_empty() {
        for agent in &mut agents {
            if last_interrupted.contains(&agent.pane_id) && !interrupted.contains(&agent.pane_id) {
                agent.status_ts = Some(now_ts);
                agent_writes.push(AgentWrite {
                    pane_id: agent.pane_id.clone(),
                    status_ts: now_ts,
                });
            }
        }
    }

    // Phase 3: Build snapshot from already-mutated agents
    let mut snapshot = build_snapshot(
        agents,
        &tmux_state.window_statuses,
        &tmux_state.pane_window_ids,
        tmux_state.active_windows,
        tmux_state.active_pane_ids,
        tmux_state.window_pane_counts,
        position,
        layout_mode,
        status_icons,
        git_statuses,
        pr_statuses,
        &sleeping_pane_ids,
    );
    snapshot.interrupted_pane_ids = interrupted.clone();

    // Phase 4: Determine runtime write side effect
    let runtime_write = if interrupted != *last_interrupted || heartbeat_due {
        Some(crate::state::RuntimeState {
            interrupted_pane_ids: interrupted.clone(),
            updated_ts: now_ts,
        })
    } else {
        None
    };

    TickOutput {
        snapshot,
        agent_writes,
        runtime_write,
        next_interrupted: interrupted,
    }
}

/// Apply side effects computed by `compute_tick`.
/// Returns true if runtime state was written.
fn apply_tick_effects(
    output: &TickOutput,
    store: &StateStore,
    backend: &str,
    instance: &str,
) -> bool {
    for write in &output.agent_writes {
        let pane_key = crate::state::PaneKey {
            backend: backend.to_string(),
            instance: instance.to_string(),
            pane_id: write.pane_id.clone(),
        };
        if let Ok(Some(mut state)) = store.get_agent(&pane_key) {
            state.status_ts = Some(write.status_ts);
            let _ = store.upsert_agent(&state);
        }
    }

    if let Some(ref runtime) = output.runtime_write {
        let _ = store.write_runtime(backend, instance, runtime);
        true
    } else {
        false
    }
}

/// Capture pane content for working agents that need checking.
/// Skips agents already confirmed as interrupted (no I/O needed until they resume).
/// Whether a pane's output should be sampled for the interruption heuristic.
///
/// An ADE agent's `pane_id` is a namespaced reference, not a pane: asking the
/// multiplexer to capture it would hand tmux something it cannot resolve, once
/// per tick, for as long as the agent lives. The heuristic is pane-output-based
/// and does not apply to an agent another manager runs.
fn is_capturable(a: &AgentPane) -> bool {
    a.has_pane() && a.status == Some(AgentStatus::Working)
}

fn gather_captures(
    agents: &[crate::multiplexer::AgentPane],
    mux: &dyn Multiplexer,
    tracker: &InactivityTracker,
    now_ts: u64,
) -> HashMap<String, String> {
    agents
        .iter()
        .filter(|a| is_capturable(a))
        .filter(|a| !tracker.is_confirmed(&a.pane_id, a.updated_ts.unwrap_or(0)))
        // A fresh RPC heartbeat (agent status hooks fire on every tool call)
        // proves the agent is alive, so the capture-and-hash stall check is
        // unnecessary. Hookless agents never heartbeat and are always captured.
        .filter(|a| {
            now_ts.saturating_sub(a.updated_ts.unwrap_or(0)) >= INACTIVITY_TIMEOUT.as_secs()
        })
        .filter_map(|a| {
            mux.capture_pane(&a.pane_id, 5)
                .map(|content| (a.pane_id.clone(), content))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiplexer::{AgentPane, AgentStatus};
    use std::cell::RefCell;
    use std::path::PathBuf;

    mod tick_diet_tests {
        use super::*;

        #[test]
        fn a_signalled_tick_rereads_the_cached_options() {
            let now = Instant::now();
            let mut opts = TmuxOptions::new();
            assert!(opts.is_stale(false, now), "nothing read yet");
            opts.read_at = Some(now);
            assert!(!opts.is_stale(false, now), "an idle tick reuses the cache");
            assert!(opts.is_stale(true, now), "a signal forces a re-read");
        }

        #[test]
        fn an_unsignalled_change_is_picked_up_within_one_sweep() {
            let now = Instant::now();
            let mut opts = TmuxOptions::new();
            opts.read_at = Some(now);
            assert!(!opts.is_stale(false, now + SWEEP_INTERVAL / 2));
            assert!(opts.is_stale(false, now + SWEEP_INTERVAL));
        }

        #[test]
        fn an_idle_loop_waits_for_its_next_sweep() {
            // No client countdown, health log far off: nothing to poll for.
            let wait = next_loop_wait(
                Duration::from_secs(1),
                SWEEP_INTERVAL,
                None,
                Duration::from_secs(59),
            );
            assert_eq!(wait, SWEEP_INTERVAL - Duration::from_secs(1));
        }

        #[test]
        fn the_wait_never_outlasts_the_idle_exit() {
            let wait = next_loop_wait(
                Duration::ZERO,
                SWEEP_INTERVAL,
                Some(Duration::from_secs(4)),
                Duration::from_secs(59),
            );
            assert_eq!(wait, Duration::from_secs(4));
        }

        #[test]
        fn a_past_deadline_does_not_wait() {
            let wait = next_loop_wait(
                SWEEP_INTERVAL * 2,
                SWEEP_INTERVAL,
                None,
                Duration::from_secs(10),
            );
            assert_eq!(wait, Duration::ZERO);
        }

        pub(super) fn empty_snapshot() -> crate::command::sidebar::snapshot::SidebarSnapshot {
            crate::command::sidebar::snapshot::SidebarSnapshot {
                position: crate::config::SidebarPosition::Left,
                layout_mode: SidebarLayoutMode::default(),
                active_windows: HashSet::new(),
                active_pane_ids: HashSet::new(),
                window_pane_counts: HashMap::new(),
                git_statuses: HashMap::new(),
                pr_statuses: HashMap::new(),
                interrupted_pane_ids: HashSet::new(),
                sleeping_pane_ids: HashSet::new(),
                agents: Vec::new(),
                config_version: 0,
            }
        }

        /// Read one length-prefixed frame body, or None if the socket is empty.
        pub(super) fn read_frame(stream: &mut UnixStream) -> Option<Vec<u8>> {
            use std::io::Read;
            stream
                .set_read_timeout(Some(Duration::from_millis(200)))
                .unwrap();
            let mut len = [0u8; 4];
            stream.read_exact(&mut len).ok()?;
            let len = u32::from_be_bytes(len) as usize;
            let mut body = vec![0u8; len];
            stream.read_exact(&mut body).ok()?;
            Some(body)
        }

        fn no_git_requests() -> GitRequests {
            GitRequests {
                cache: Arc::new(Mutex::new(HashMap::new())),
                prioritise: Arc::new(Mutex::new(HashSet::new())),
            }
        }

        pub(super) fn wait_for_clients(server: &SocketServer, want: usize) {
            for _ in 0..200 {
                if server.client_count() == want {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(server.client_count(), want);
        }

        #[test]
        fn an_unchanged_snapshot_is_not_pushed_twice() {
            let dir = tempfile::tempdir().unwrap();
            let sock = dir.path().join("d.sock");
            let server = SocketServer::bind(&sock, no_git_requests()).unwrap();
            let mut client = UnixStream::connect(&sock).unwrap();
            // Let the accept thread register the connection.
            wait_for_clients(&server, 1);

            let snapshot = empty_snapshot();
            assert!(server.broadcast(&snapshot), "first push goes out");
            assert!(
                !server.broadcast(&snapshot.clone()),
                "an identical snapshot is not pushed again"
            );
            assert_eq!(
                server.client_count(),
                1,
                "the skipped push must not reap a live client"
            );
            let mut changed = snapshot.clone();
            changed.config_version = 1;
            assert!(server.broadcast(&changed), "a changed field pushes again");

            assert!(read_frame(&mut client).is_some());
            assert!(read_frame(&mut client).is_some());
            assert!(
                read_frame(&mut client).is_none(),
                "exactly two frames, not three"
            );
        }

        #[test]
        fn only_a_closed_peer_counts_as_gone() {
            // An interrupted syscall (the daemon is SIGUSR1'd by every
            // agent-status hook) must not read as a hangup.
            let (ours, theirs) = UnixStream::pair().unwrap();
            assert!(!client_is_gone(&ours), "the peer is still open");
            drop(theirs);
            assert!(client_is_gone(&ours));
        }

        #[test]
        fn a_client_with_a_request_in_flight_is_not_gone() {
            // Clients can speak now, so readable no longer implies hangup.
            let (ours, mut theirs) = UnixStream::pair().unwrap();
            theirs.write_all(b"\0\0\0\x02{}").unwrap();
            assert!(
                !client_is_gone(&ours),
                "a pending request must not read as a hangup"
            );
        }

        #[test]
        fn a_closed_client_is_gone_even_with_bytes_left_unread() {
            // A desynced stream leaves bytes queued forever. If those read as
            // a busy client the daemon never reaches an idle client count and
            // never exits.
            let (ours, mut theirs) = UnixStream::pair().unwrap();
            theirs.write_all(b"\0\0\0\x02{}").unwrap();
            drop(theirs);
            assert!(client_is_gone(&ours));
        }

        #[test]
        fn a_dead_client_with_queued_bytes_is_pruned() {
            let dir = tempfile::tempdir().unwrap();
            let sock = dir.path().join("d.sock");
            let server = SocketServer::bind(&sock, no_git_requests()).unwrap();
            let mut client = UnixStream::connect(&sock).unwrap();
            wait_for_clients(&server, 1);

            // An over-cap length prefix: `serve_requests` stops reading and
            // the body stays queued on the daemon's side.
            client
                .write_all(
                    &((crate::command::sidebar::protocol::MAX_FRAME + 1) as u32).to_be_bytes(),
                )
                .unwrap();
            client.write_all(b"{}").unwrap();
            drop(client);

            for _ in 0..200 {
                server.prune_dead_clients();
                if server.client_count() == 0 {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
            panic!("the daemon still counts a closed client, so it can never idle out");
        }

        #[test]
        fn a_new_client_gets_the_current_snapshot_on_connect() {
            let dir = tempfile::tempdir().unwrap();
            let sock = dir.path().join("d.sock");
            let server = SocketServer::bind(&sock, no_git_requests()).unwrap();
            server.broadcast(&empty_snapshot());

            let mut client = UnixStream::connect(&sock).unwrap();
            assert!(
                read_frame(&mut client).is_some(),
                "the cached snapshot arrives without waiting for a tick"
            );
        }
    }

    mod request_tests {
        use super::tick_diet_tests::{empty_snapshot, read_frame, wait_for_clients};
        use super::*;
        use crate::command::sidebar::protocol::{
            DaemonRequest, DaemonResponse, RequestPayload, ResponsePayload, write_frame,
        };

        fn fingerprinted(lines_added: usize) -> GitStatus {
            GitStatus {
                lines_added,
                head_oid: Some("head-oid".to_string()),
                base_oid: Some("base-oid".to_string()),
                cached_at: Some(42),
                ..Default::default()
            }
        }

        /// A server whose cache holds `/wt/a`, plus its prioritise queue.
        fn server_with_cache(sock: &Path) -> (SocketServer, PrioritiseQueue) {
            let cache: GitCache = Arc::new(Mutex::new(HashMap::from([(
                PathBuf::from("/wt/a"),
                fingerprinted(3),
            )])));
            let prioritise: PrioritiseQueue = Arc::new(Mutex::new(HashSet::new()));
            let server = SocketServer::bind(
                sock,
                GitRequests {
                    cache,
                    prioritise: prioritise.clone(),
                },
            )
            .unwrap();
            (server, prioritise)
        }

        fn ask(stream: &mut UnixStream, id: u64, paths: &[&str], prioritise: bool) {
            write_frame(
                stream,
                &DaemonRequest {
                    id,
                    payload: RequestPayload::GetGitStatuses {
                        paths: paths.iter().map(PathBuf::from).collect(),
                        prioritise,
                    },
                },
            )
            .unwrap();
        }

        /// Read frames until one parses as a response, or give up.
        fn next_response(stream: &mut UnixStream) -> Option<DaemonResponse> {
            for _ in 0..10 {
                let frame = read_frame(stream)?;
                if let Ok(resp) = serde_json::from_slice::<DaemonResponse>(&frame) {
                    return Some(resp);
                }
            }
            None
        }

        fn statuses(resp: &DaemonResponse) -> &HashMap<PathBuf, GitStatus> {
            match &resp.payload {
                ResponsePayload::GitStatuses(s) => s,
            }
        }

        #[test]
        fn known_paths_are_answered_from_the_cache_with_their_fingerprint() {
            let dir = tempfile::tempdir().unwrap();
            let sock = dir.path().join("d.sock");
            let (server, _) = server_with_cache(&sock);
            let mut client = UnixStream::connect(&sock).unwrap();
            wait_for_clients(&server, 1);

            ask(&mut client, 9, &["/wt/a", "/wt/unknown"], false);
            let resp = next_response(&mut client).expect("a reply");

            assert_eq!(resp.id, 9, "the reply carries the request id");
            let got = statuses(&resp);
            assert_eq!(got.len(), 1, "an unknown path is omitted, not an error");
            let a = &got[&PathBuf::from("/wt/a")];
            assert_eq!(a.lines_added, 3);
            // The consumer needs the fingerprint to reuse this as `prev`;
            // only the snapshot path strips it.
            assert_eq!(a.head_oid.as_deref(), Some("head-oid"));
            assert_eq!(a.base_oid.as_deref(), Some("base-oid"));
            assert_eq!(a.cached_at, Some(42));
        }

        #[test]
        fn a_hint_is_drained_down_to_the_paths_the_worker_tracks() {
            let queue: PrioritiseQueue = Arc::new(Mutex::new(HashSet::from([
                PathBuf::from("/wt/a"),
                PathBuf::from("/wt/elsewhere"),
            ])));
            let tracked = HashMap::from([(PathBuf::from("/wt/a"), false)]);

            assert_eq!(
                take_prioritised(&queue, &tracked),
                vec![PathBuf::from("/wt/a")]
            );
            assert!(
                take_prioritised(&queue, &tracked).is_empty(),
                "the queue is drained, so one hint cannot refresh forever"
            );
        }

        #[test]
        fn a_prioritise_hint_queues_the_paths() {
            let dir = tempfile::tempdir().unwrap();
            let sock = dir.path().join("d.sock");
            let (server, prioritise) = server_with_cache(&sock);
            let mut client = UnixStream::connect(&sock).unwrap();
            wait_for_clients(&server, 1);

            ask(&mut client, 1, &["/wt/a", "/wt/b"], true);
            next_response(&mut client).expect("a reply");

            let queued = prioritise.lock().unwrap().clone();
            assert_eq!(
                queued,
                HashSet::from([PathBuf::from("/wt/a"), PathBuf::from("/wt/b")]),
                "the hint names every requested path, cached or not"
            );
        }

        #[test]
        fn a_snapshot_pushed_mid_request_interleaves_with_the_reply() {
            let dir = tempfile::tempdir().unwrap();
            let sock = dir.path().join("d.sock");
            let (server, _) = server_with_cache(&sock);
            let mut client = UnixStream::connect(&sock).unwrap();
            wait_for_clients(&server, 1);

            ask(&mut client, 4, &["/wt/a"], false);
            assert!(server.broadcast(&empty_snapshot()));

            let mut saw_snapshot = false;
            let mut saw_reply = false;
            while let Some(frame) = read_frame(&mut client) {
                if let Ok(resp) = serde_json::from_slice::<DaemonResponse>(&frame) {
                    assert_eq!(resp.id, 4);
                    saw_reply = true;
                } else {
                    serde_json::from_slice::<crate::command::sidebar::snapshot::SidebarSnapshot>(
                        &frame,
                    )
                    .expect("a frame is either a reply or a snapshot");
                    saw_snapshot = true;
                }
                if saw_snapshot && saw_reply {
                    break;
                }
            }
            assert!(
                saw_reply && saw_snapshot,
                "both frames arrived, in any order"
            );
        }

        #[test]
        fn a_malformed_frame_is_ignored_without_dropping_the_connection() {
            let dir = tempfile::tempdir().unwrap();
            let sock = dir.path().join("d.sock");
            let (server, _) = server_with_cache(&sock);
            let mut client = UnixStream::connect(&sock).unwrap();
            wait_for_clients(&server, 1);

            let junk = b"not json at all";
            client
                .write_all(&(junk.len() as u32).to_be_bytes())
                .unwrap();
            client.write_all(junk).unwrap();
            // A well-formed frame with an unknown payload variant is equally
            // unparseable, and equally harmless.
            write_frame(
                &mut client,
                &serde_json::json!({"id": 1, "payload": {"Whatever": {}}}),
            )
            .unwrap();

            ask(&mut client, 2, &["/wt/a"], false);
            let resp = next_response(&mut client).expect("the connection still serves requests");
            assert_eq!(resp.id, 2);
            assert_eq!(server.client_count(), 1, "the client was not reaped");
        }

        #[test]
        fn a_client_that_disconnects_mid_request_is_reaped() {
            let dir = tempfile::tempdir().unwrap();
            let sock = dir.path().join("d.sock");
            let (server, _) = server_with_cache(&sock);
            let mut client = UnixStream::connect(&sock).unwrap();
            wait_for_clients(&server, 1);

            ask(&mut client, 1, &["/wt/a"], false);
            drop(client);

            // The reply write and the EOF race; either way the peer is gone
            // once the handler has drained the request.
            for _ in 0..200 {
                server.prune_dead_clients();
                if server.client_count() == 0 {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(server.client_count(), 0);
            assert!(
                server.broadcast(&empty_snapshot()),
                "the server still serves other clients"
            );
        }
    }

    mod git_sweep_interval_tests {
        use super::*;

        #[cfg(not(target_os = "macos"))]
        #[test]
        fn a_watched_linux_worktree_sweeps_every_ten_seconds() {
            assert_eq!(git_sweep_interval(true), Duration::from_secs(10));
        }

        #[test]
        fn a_missing_watcher_polls_every_two_seconds() {
            assert_eq!(git_sweep_interval(false), Duration::from_secs(2));
        }
    }

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
            window_cmd: None,
            agent_command: None,
            agent_kind: None,
            pipeline_node_title: None,
            activity: None,
            pane_pid: 0,
            runtime: None,
        }
    }

    fn done_agent(pane_id: &str) -> AgentPane {
        AgentPane {
            status: Some(AgentStatus::Done),
            ..working_agent(pane_id, 1)
        }
    }

    #[test]
    fn pr_path_cache_records_branch() {
        let path = PathBuf::from("/repo");
        let repo_root = PathBuf::from("/repo");
        let summary = PrSummary {
            number: 123,
            title: "test".to_string(),
            state: "OPEN".to_string(),
            is_draft: false,
            checks: None,
            check_meta: None,
            url: None,
        };
        let entries = vec![PrWorkerPath {
            path: path.clone(),
            branch: "feature".to_string(),
        }];
        let repo_roots = HashMap::from([(path.clone(), repo_root.clone())]);
        let repo_cache =
            HashMap::from([(repo_root, HashMap::from([("feature".to_string(), summary)]))]);
        let path_cache: PrPathCache = Arc::new(Mutex::new(HashMap::new()));
        let dirty_flag = Arc::new(AtomicBool::new(false));
        let (wake_tx, _wake_rx) = std::sync::mpsc::sync_channel(1);

        publish_pr_path_cache(
            &entries,
            &repo_roots,
            &repo_cache,
            &path_cache,
            &dirty_flag,
            &wake_tx,
        );

        let cache = path_cache.lock().unwrap();
        let entry = cache.get(&path).unwrap();
        assert_eq!(entry.branch, "feature");
        assert_eq!(entry.summary.number, 123);
        assert!(dirty_flag.load(Ordering::Relaxed));
    }

    #[test]
    fn a_failed_manager_listing_keeps_the_last_good_rows() {
        let mut ade = AdeAgents::new();
        let t0 = Instant::now();
        let good = vec![working_agent("paseo:a", 0)];
        assert_eq!(ade.apply(t0, good.clone(), vec![]).len(), 1);
        // The daemon is restarting: the rows stay, the problem is remembered.
        let kept = ade.apply(t0, vec![], vec![("paseo".into(), "ECONNREFUSED".into())]);
        assert_eq!(kept.len(), 1);
        assert!(ade.last_problem.is_some());
        // It is back: its rows replace the retained ones.
        assert!(ade.apply(t0, vec![], vec![]).is_empty());
        assert!(ade.last_problem.is_none());
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
    fn pane_less_agents_are_never_captured() {
        // An ADE agent's `pane_id` is a namespaced reference, not a pane. Asking
        // the multiplexer to capture it would send tmux something it cannot
        // resolve, once per tick, for as long as the agent lives.
        let mut ade = working_agent("paseo:agt_1", 10);
        ade.runtime = Some("paseo".to_string());
        assert!(!ade.has_pane());
        assert_eq!(ade.runtime_name(), "paseo");

        let local = working_agent("%1", 10);
        assert!(local.has_pane());
        assert_eq!(local.runtime_name(), "local");

        assert!(
            !is_capturable(&ade),
            "pane-less agents must not reach the multiplexer"
        );
        assert!(is_capturable(&local));
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

    // ── Tick-level tests (tracker + state store + runtime) ──────────────

    mod tick {
        use super::*;
        use crate::config::StatusIcons;
        use crate::multiplexer::AgentStatus;
        use crate::state::{PaneKey, StateStore};

        const BACKEND: &str = "tmux";
        const INSTANCE: &str = "test";

        fn test_store() -> (StateStore, tempfile::TempDir) {
            let dir = tempfile::TempDir::new().unwrap();
            let store = StateStore::with_path(dir.path().to_path_buf()).unwrap();
            (store, dir)
        }

        fn pane_key(pane_id: &str) -> PaneKey {
            PaneKey {
                backend: BACKEND.to_string(),
                instance: INSTANCE.to_string(),
                pane_id: pane_id.to_string(),
            }
        }

        fn seed_agent(store: &StateStore, pane_id: &str, status_ts: u64, updated_ts: u64) {
            let state = crate::state::AgentState {
                agent_id: uuid::Uuid::new_v4().to_string(),
                pane_key: pane_key(pane_id),
                workdir: PathBuf::from("/tmp"),
                status: Some(AgentStatus::Working),
                status_ts: Some(status_ts),
                pane_title: None,
                provider_session_id: None,
                pane_pid: 1,
                command: "node".to_string(),
                updated_ts,
                window_name: None,
                session_name: None,
                boot_id: None,
                agent_kind: None,
                sandbox_id: None,
                checkpoint_path: None,
                checkpoint_ts: None,
                pipeline_node_id: None,
                pipeline_node_title: None,
                activity: None,
                runtime: None,
                completion: None,
            };
            store.upsert_agent(&state).unwrap();
        }

        fn do_tick(
            tracker: &mut InactivityTracker,
            last: &mut HashSet<String>,
            agents: Vec<crate::multiplexer::AgentPane>,
            captures: HashMap<String, String>,
            now: Instant,
            now_ts: u64,
        ) -> TickOutput {
            let output = compute_tick(
                TickInput {
                    agents,
                    tmux_state: TmuxState {
                        window_statuses: HashMap::new(),
                        active_windows: HashSet::new(),
                        pane_window_ids: HashMap::new(),
                        active_pane_ids: HashSet::new(),
                        window_pane_counts: HashMap::new(),
                        server_gone: false,
                    },
                    captured_panes: captures,
                    now,
                    now_ts,
                    position: SidebarPosition::Left,
                    layout_mode: SidebarLayoutMode::default(),
                    git_statuses: HashMap::new(),
                    pr_statuses: HashMap::new(),
                    sleeping_pane_ids: HashSet::new(),
                },
                tracker,
                last,
                &StatusIcons::default(),
                false,
            );
            // Commit state like the daemon loop does after apply_tick_effects
            *last = output.next_interrupted.clone();
            output
        }

        fn cap(content: &str) -> HashMap<String, String> {
            HashMap::from([("%1".to_string(), content.to_string())])
        }

        fn cap2(content: &str) -> HashMap<String, String> {
            HashMap::from([
                ("%1".to_string(), content.to_string()),
                ("%2".to_string(), content.to_string()),
            ])
        }

        #[test]
        fn resumed_agent_gets_status_ts_reset() {
            let (store, _dir) = test_store();
            seed_agent(&store, "%1", 100, 1);

            let mut tracker = InactivityTracker::new(Duration::from_secs(10));
            let mut last = HashSet::new();
            let t0 = Instant::now();

            // Tick 1: start observing
            do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 1)],
                cap("hello"),
                t0,
                1000,
            );

            // Tick 2: interrupted
            let output = do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 1)],
                cap("hello"),
                t0 + Duration::from_secs(11),
                1011,
            );
            assert!(output.snapshot.interrupted_pane_ids.contains("%1"));

            // Tick 3: agent resumes (updated_ts 1 -> 2)
            let output = do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 2)],
                cap("hello"),
                t0 + Duration::from_secs(12),
                1012,
            );
            assert!(output.snapshot.interrupted_pane_ids.is_empty());

            // Snapshot has corrected status_ts (no stale one-tick race)
            let agent = output
                .snapshot
                .agents
                .iter()
                .find(|a| a.pane_id == "%1")
                .unwrap();
            assert_eq!(agent.status_ts, Some(1012));

            // Side effect says to write it to disk
            assert_eq!(output.agent_writes.len(), 1);
            assert_eq!(output.agent_writes[0].status_ts, 1012);

            // Apply effects and verify store
            apply_tick_effects(&output, &store, BACKEND, INSTANCE);
            assert_eq!(
                store.get_agent(&pane_key("%1")).unwrap().unwrap().status_ts,
                Some(1012)
            );
        }

        #[test]
        fn only_resumed_agent_gets_reset() {
            let (store, _dir) = test_store();
            seed_agent(&store, "%1", 100, 1);
            seed_agent(&store, "%2", 200, 1);

            let mut tracker = InactivityTracker::new(Duration::from_secs(10));
            let mut last = HashSet::new();
            let t0 = Instant::now();

            let agents = vec![working_agent("%1", 1), working_agent("%2", 1)];

            // Tick 1 + 2: both interrupted
            do_tick(
                &mut tracker,
                &mut last,
                agents.clone(),
                cap2("hello"),
                t0,
                1000,
            );
            do_tick(
                &mut tracker,
                &mut last,
                agents,
                cap2("hello"),
                t0 + Duration::from_secs(11),
                1011,
            );

            // Tick 3: only %1 resumes
            let mixed = vec![working_agent("%1", 2), working_agent("%2", 1)];
            let output = do_tick(
                &mut tracker,
                &mut last,
                mixed,
                cap2("hello"),
                t0 + Duration::from_secs(12),
                1012,
            );

            // Only %1 in agent_writes
            assert_eq!(output.agent_writes.len(), 1);
            assert_eq!(output.agent_writes[0].pane_id, "%1");

            // Apply and verify
            apply_tick_effects(&output, &store, BACKEND, INSTANCE);
            assert_eq!(
                store.get_agent(&pane_key("%1")).unwrap().unwrap().status_ts,
                Some(1012)
            );
            assert_eq!(
                store.get_agent(&pane_key("%2")).unwrap().unwrap().status_ts,
                Some(200)
            );
        }

        #[test]
        fn runtime_file_reflects_interrupted_set() {
            let (store, _dir) = test_store();

            let mut tracker = InactivityTracker::new(Duration::from_secs(10));
            let mut last = HashSet::new();
            let t0 = Instant::now();

            // Tick 1: not interrupted yet
            let output = do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 1)],
                cap("hello"),
                t0,
                1000,
            );
            apply_tick_effects(&output, &store, BACKEND, INSTANCE);

            // Tick 2: interrupted
            let output = do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 1)],
                cap("hello"),
                t0 + Duration::from_secs(11),
                1011,
            );
            apply_tick_effects(&output, &store, BACKEND, INSTANCE);
            assert!(
                store
                    .read_runtime(BACKEND, INSTANCE)
                    .interrupted_pane_ids
                    .contains("%1")
            );

            // Tick 3: resumes
            let output = do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 2)],
                cap("hello"),
                t0 + Duration::from_secs(12),
                1012,
            );
            apply_tick_effects(&output, &store, BACKEND, INSTANCE);
            assert!(
                store
                    .read_runtime(BACKEND, INSTANCE)
                    .interrupted_pane_ids
                    .is_empty()
            );
        }

        #[test]
        fn missing_agent_file_does_not_panic() {
            let (store, _dir) = test_store();

            let mut tracker = InactivityTracker::new(Duration::from_secs(10));
            let mut last = HashSet::new();
            let t0 = Instant::now();

            // Tick 1 + 2: become interrupted
            do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 1)],
                cap("hello"),
                t0,
                1000,
            );
            do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 1)],
                cap("hello"),
                t0 + Duration::from_secs(11),
                1011,
            );

            // Tick 3: resume with no agent file - should not panic
            let output = do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 2)],
                cap("hello"),
                t0 + Duration::from_secs(12),
                1012,
            );
            assert!(output.snapshot.interrupted_pane_ids.is_empty());
            apply_tick_effects(&output, &store, BACKEND, INSTANCE);
        }

        #[test]
        fn snapshot_has_correct_status_ts_on_resume_tick() {
            // Proves the one-tick race is structurally impossible: agents
            // are mutated before build_snapshot, not patched after.
            let mut tracker = InactivityTracker::new(Duration::from_secs(10));
            let mut last = HashSet::new();
            let t0 = Instant::now();

            do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 1)],
                cap("hello"),
                t0,
                1000,
            );
            do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 1)],
                cap("hello"),
                t0 + Duration::from_secs(11),
                1011,
            );

            // Resume tick: snapshot must have the fresh status_ts, not the stale 100
            let output = do_tick(
                &mut tracker,
                &mut last,
                vec![working_agent("%1", 2)],
                cap("hello"),
                t0 + Duration::from_secs(12),
                1012,
            );
            let agent = output
                .snapshot
                .agents
                .iter()
                .find(|a| a.pane_id == "%1")
                .unwrap();
            assert_eq!(agent.status_ts, Some(1012));
            assert!(!output.snapshot.interrupted_pane_ids.contains("%1"));
        }
    }

    #[cfg(target_os = "linux")]
    mod freeze_tracker_tests {
        use super::*;
        use std::collections::HashSet;

        /// Read a process's state character from /proc/{pid}/status.
        /// Returns 'T' (stopped), 'S' (sleeping), 'R' (running), etc.
        fn proc_state(pid: u32) -> char {
            let content =
                std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
            for line in content.lines() {
                if let Some(rest) = line.strip_prefix("State:") {
                    return rest.trim().chars().next().unwrap_or('?');
                }
            }
            '?'
        }

        /// A `bash -c 'sleep 9999 & wait $!'` fixture: bash with a real child
        /// for `proc_descendants()` to find and `FreezeTracker` to SIGSTOP.
        ///
        /// It owns a process group and kills the whole group on `Drop`. Without
        /// that, a panicking test leaks the `sleep`, which keeps the test
        /// runner's inherited stdout open: cargo exits, and anything reading
        /// that pipe to EOF (a shell pipeline, CI, an agent) blocks forever.
        /// Null stdio keeps it out of the pipe in the first place.
        struct ShellWithChild(std::process::Child);

        impl ShellWithChild {
            fn spawn() -> Self {
                use std::os::unix::process::CommandExt;
                Self(
                    std::process::Command::new("bash")
                        .args(["-c", "sleep 9999 & wait $!"])
                        .process_group(0)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .expect("failed to spawn bash"),
                )
            }

            fn id(&self) -> u32 {
                self.0.id()
            }
        }

        impl Drop for ShellWithChild {
            fn drop(&mut self) {
                // `process_group(0)` made bash its own group leader, so its pid
                // is the pgid. SIGCONT first in case the test left the group
                // frozen, then SIGKILL the group so `sleep` dies with bash.
                let pgid = self.0.id() as i32;
                unsafe {
                    libc::killpg(pgid, libc::SIGCONT);
                    libc::killpg(pgid, libc::SIGKILL);
                }
                let _ = self.0.wait();
            }
        }

        /// Wait until the given PID has at least one child in /proc (bash needs
        /// a moment to fork sleep after startup).
        fn wait_for_children(pid: u32) {
            for _ in 0..100 {
                if !proc_descendants(pid).is_empty() {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            panic!("timed out waiting for child processes of pid {pid}");
        }

        fn done_pane(pane_id: &str, shell_pid: u32) -> AgentPane {
            AgentPane {
                session: String::new(),
                window_name: "test-window".to_string(),
                pane_id: pane_id.to_string(),
                window_id: "@99".to_string(),
                path: std::path::PathBuf::new(),
                pane_title: None,
                status: Some(AgentStatus::Done),
                status_ts: Some(0),
                updated_ts: None,
                window_cmd: None,
                agent_command: None,
                agent_kind: None,
                pipeline_node_title: None,
                activity: None,
                pane_pid: shell_pid,
                runtime: None,
            }
        }

        #[test]
        fn freeze_stops_idle_done_agent() {
            let child = ShellWithChild::spawn();
            let shell_pid = child.id();
            wait_for_children(shell_pid);

            let mut tracker = FreezeTracker::new();
            let agents = vec![done_pane("%freeze_1", shell_pid)];
            let no_active: HashSet<String> = HashSet::new();

            // after_idle=0, now_ts=9999 → idle_secs=9999 ≥ 0 → freeze immediately
            tracker.tick(&agents, &no_active, std::time::Duration::ZERO, 9999);

            // All child processes of the shell should be stopped (state 'T')
            let children = proc_descendants(shell_pid);
            assert!(!children.is_empty(), "expected child processes to exist");
            for &cpid in &children {
                assert_eq!(
                    proc_state(cpid),
                    'T',
                    "child {cpid} should be stopped after freeze"
                );
            }
        }

        #[test]
        fn thaw_on_pane_focus() {
            let child = ShellWithChild::spawn();
            let shell_pid = child.id();
            wait_for_children(shell_pid);
            let children = proc_descendants(shell_pid);

            let mut tracker = FreezeTracker::new();
            let agents = vec![done_pane("%freeze_2", shell_pid)];
            let no_active: HashSet<String> = HashSet::new();

            // Freeze
            tracker.tick(&agents, &no_active, std::time::Duration::ZERO, 9999);
            for &cpid in &children {
                assert_eq!(proc_state(cpid), 'T', "should be stopped after freeze");
            }

            // Thaw by making the pane active (simulates pane-focus-in)
            let mut active: HashSet<String> = HashSet::new();
            active.insert("%freeze_2".to_string());
            tracker.tick(&agents, &active, std::time::Duration::ZERO, 9999);

            for &cpid in &children {
                let state = proc_state(cpid);
                assert!(
                    matches!(state, 'S' | 'R' | 'D'),
                    "child {cpid} should be running after thaw-on-focus, got '{state}'"
                );
            }
        }

        #[test]
        fn thaw_on_status_change_to_working() {
            let child = ShellWithChild::spawn();
            let shell_pid = child.id();
            wait_for_children(shell_pid);
            let children = proc_descendants(shell_pid);

            let mut tracker = FreezeTracker::new();
            let agents_done = vec![done_pane("%freeze_3", shell_pid)];
            let no_active: HashSet<String> = HashSet::new();

            tracker.tick(&agents_done, &no_active, std::time::Duration::ZERO, 9999);
            for &cpid in &children {
                assert_eq!(proc_state(cpid), 'T', "should be stopped");
            }

            // Status changes to Working
            let agents_working = vec![AgentPane {
                status: Some(AgentStatus::Working),
                ..agents_done[0].clone()
            }];
            tracker.tick(&agents_working, &no_active, std::time::Duration::ZERO, 9999);

            for &cpid in &children {
                let state = proc_state(cpid);
                assert!(
                    matches!(state, 'S' | 'R' | 'D'),
                    "child {cpid} should be running after status→Working, got '{state}'"
                );
            }
        }

        #[test]
        fn thaw_all_unfreezes_on_shutdown() {
            let child = ShellWithChild::spawn();
            let shell_pid = child.id();
            wait_for_children(shell_pid);
            let children = proc_descendants(shell_pid);

            let mut tracker = FreezeTracker::new();
            let agents = vec![done_pane("%freeze_4", shell_pid)];
            let no_active: HashSet<String> = HashSet::new();

            tracker.tick(&agents, &no_active, std::time::Duration::ZERO, 9999);
            for &cpid in &children {
                assert_eq!(proc_state(cpid), 'T', "should be stopped");
            }

            tracker.thaw_all();

            for &cpid in &children {
                let state = proc_state(cpid);
                assert!(
                    matches!(state, 'S' | 'R' | 'D'),
                    "child {cpid} still stopped after thaw_all, got '{state}'"
                );
            }
        }

        #[test]
        fn frozen_entry_stores_window_metadata() {
            let child = ShellWithChild::spawn();
            let shell_pid = child.id();
            wait_for_children(shell_pid);

            let mut tracker = FreezeTracker::new();
            let agents = vec![done_pane("%freeze_5", shell_pid)];
            let no_active: HashSet<String> = HashSet::new();

            tracker.tick(&agents, &no_active, std::time::Duration::ZERO, 9999);

            let entry = tracker
                .frozen
                .get("%freeze_5")
                .expect("pane should be in frozen map");
            assert_eq!(entry.original_window_name, "test-window");
            assert_eq!(entry.window_id, "@99");
            assert!(
                !entry.child_pids.is_empty(),
                "should have recorded child PIDs"
            );
        }
    }
}
