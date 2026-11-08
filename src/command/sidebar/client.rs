//! Unix socket client for receiving snapshots from the muxix daemon.

use std::collections::HashMap;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::git::GitStatus;

use super::protocol::{
    DaemonRequest, DaemonResponse, RequestPayload, ResponsePayload, read_frame, write_frame,
};
use super::snapshot::SidebarSnapshot;

type Latest = Arc<Mutex<Option<SidebarSnapshot>>>;
type Requests = Arc<Mutex<RequestState>>;

/// The request side of the connection. Replies arrive on the same socket as
/// pushed snapshots, so the reader thread routes them here by id.
#[derive(Default)]
struct RequestState {
    /// Write half of the live connection; `None` while disconnected.
    writer: Option<UnixStream>,
    next_id: u64,
    waiting: HashMap<u64, mpsc::Sender<ResponsePayload>>,
}

/// Handle for taking the latest coalesced snapshot.
pub struct SnapshotHandle {
    latest: Latest,
    requests: Requests,
}

impl SnapshotHandle {
    /// Take the latest snapshot (if any). Returns None if no new data.
    pub fn take(&self) -> Option<SidebarSnapshot> {
        self.latest.lock().unwrap().take()
    }

    /// Ask the daemon for the git status it has cached for `paths`, optionally
    /// hinting that it should refresh them early.
    ///
    /// `None` means no answer — not connected, the daemon went away, or it did
    /// not reply in time. Callers compute the statuses themselves in that case.
    /// Paths the daemon knows nothing about are absent from the map.
    // Used by the dashboard once it consumes the daemon (group 5).
    #[allow(dead_code)]
    pub fn request_git_statuses(
        &self,
        paths: Vec<PathBuf>,
        prioritise: bool,
        timeout: Duration,
    ) -> Option<HashMap<PathBuf, GitStatus>> {
        let (tx, rx) = mpsc::channel();
        let id = {
            // The frame is written under the lock: two concurrent requests
            // must not interleave on the socket.
            let mut state = self.requests.lock().unwrap();
            let mut writer = state.writer.as_ref()?.try_clone().ok()?;
            state.next_id += 1;
            let id = state.next_id;
            let request = DaemonRequest {
                id,
                payload: RequestPayload::GetGitStatuses { paths, prioritise },
            };
            if write_frame(&mut writer, &request).is_err() {
                return None;
            }
            state.waiting.insert(id, tx);
            id
        };
        let reply = rx.recv_timeout(timeout).ok();
        self.requests.lock().unwrap().waiting.remove(&id);
        reply.map(|ResponsePayload::GitStatuses(statuses)| statuses)
    }
}

/// Connect to daemon socket. Spawns background reader thread that overwrites
/// the latest snapshot and sends a wake signal on each update.
///
/// Returns the handle for taking snapshots. The caller should select on
/// `wake_rx` to know when new data is available.
pub fn connect(socket_path: &Path, wake_tx: mpsc::SyncSender<()>) -> SnapshotHandle {
    let latest: Latest = Arc::new(Mutex::new(None));
    let latest_clone = latest.clone();
    let requests: Requests = Arc::new(Mutex::new(RequestState::default()));
    let requests_clone = requests.clone();
    let path = socket_path.to_path_buf();

    thread::spawn(move || {
        connection_loop(&path, &latest_clone, &requests_clone, &wake_tx);
    });

    SnapshotHandle { latest, requests }
}

fn connection_loop(
    path: &Path,
    latest: &Latest,
    requests: &Requests,
    wake_tx: &mpsc::SyncSender<()>,
) {
    let min_backoff = Duration::from_millis(50);
    let max_backoff = Duration::from_secs(2);
    // PID-based jitter to prevent 12 clients from phase-locking reconnects
    let jitter = Duration::from_millis((std::process::id() % 100) as u64);
    let mut backoff = min_backoff;

    loop {
        let connected_at = Instant::now();

        if let Ok(stream) = UnixStream::connect(path) {
            backoff = min_backoff;
            // Publish the write half so requests can go out, and withdraw it
            // when the connection ends so an in-flight request fails fast
            // instead of waiting out its timeout.
            if let Ok(writer) = stream.try_clone() {
                let _ = writer.set_write_timeout(Some(Duration::from_millis(500)));
                requests.lock().unwrap().writer = Some(writer);
            }
            let ended = read_loop(stream, latest, requests, wake_tx);
            {
                let mut state = requests.lock().unwrap();
                state.writer = None;
                state.waiting.clear();
            }
            if ended.is_err() {
                break; // Wake channel closed, main thread exited
            }
            // Only reset backoff if connection was stable (lasted >5s).
            // Short-lived connections (daemon accept-then-close) keep
            // exponential backoff to prevent synchronized churn.
            if connected_at.elapsed() <= Duration::from_secs(5) {
                backoff = (backoff * 2).min(max_backoff);
            }
        }

        thread::sleep(backoff + jitter);
    }
}

/// Returns Err if the wake channel is closed (main thread exited).
fn read_loop(
    mut stream: UnixStream,
    latest: &Latest,
    requests: &Requests,
    wake_tx: &mpsc::SyncSender<()>,
) -> Result<(), mpsc::SendError<()>> {
    loop {
        // None covers socket closed and a corrupt length; either way, reconnect.
        let Some(buf) = read_frame(&mut stream) else {
            return Ok(());
        };

        // A reply is tagged with the id of the request it answers; everything
        // else on this socket is a pushed snapshot.
        if let Ok(response) = serde_json::from_slice::<DaemonResponse>(&buf) {
            if let Some(waiter) = requests.lock().unwrap().waiting.remove(&response.id) {
                let _ = waiter.send(response.payload);
            }
            continue;
        }

        if let Ok(snapshot) = serde_json::from_slice::<SidebarSnapshot>(&buf) {
            *latest.lock().unwrap() = Some(snapshot);
            // try_send: if a wake is already pending, skip (coalesces)
            // Full = no-op (wake already queued), Disconnected = main exited
            if let Err(mpsc::TrySendError::Disconnected(())) = wake_tx.try_send(()) {
                return Err(mpsc::SendError(()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::io::Write;
    use std::os::unix::net::UnixListener;

    fn empty_snapshot() -> SidebarSnapshot {
        SidebarSnapshot {
            position: crate::config::SidebarPosition::Left,
            layout_mode: super::super::app::SidebarLayoutMode::default(),
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

    /// A daemon stand-in: pushes one snapshot on connect, then answers
    /// `GetGitStatuses` with one entry for `/wt/a`. After `replies` answers it
    /// shuts down, closing the connection as an exiting daemon would.
    fn fake_daemon(listener: UnixListener, replies: usize) {
        thread::spawn(move || {
            let mut left = replies;
            while let Ok((mut stream, _)) = listener.accept() {
                let _ = write_frame(&mut stream, &empty_snapshot());
                let mut reader = stream.try_clone().unwrap();
                while let Some(frame) = read_frame(&mut reader) {
                    let Ok(request) = serde_json::from_slice::<DaemonRequest>(&frame) else {
                        continue;
                    };
                    let statuses = HashMap::from([(
                        PathBuf::from("/wt/a"),
                        GitStatus {
                            lines_added: 7,
                            head_oid: Some("head-oid".to_string()),
                            ..Default::default()
                        },
                    )]);
                    let _ = write_frame(
                        &mut stream,
                        &DaemonResponse {
                            id: request.id,
                            payload: ResponsePayload::GitStatuses(statuses),
                        },
                    );
                    left -= 1;
                    if left == 0 {
                        return;
                    }
                }
            }
        });
    }

    /// Requests race the background connect, so retry until the socket is up.
    fn ask_until_answered(handle: &SnapshotHandle) -> Option<HashMap<PathBuf, GitStatus>> {
        for _ in 0..100 {
            if let Some(statuses) = handle.request_git_statuses(
                vec![PathBuf::from("/wt/a")],
                true,
                Duration::from_millis(500),
            ) {
                return Some(statuses);
            }
            thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn a_reply_is_correlated_while_snapshots_keep_arriving() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        fake_daemon(UnixListener::bind(&sock).unwrap(), usize::MAX);

        let (wake_tx, _wake_rx) = mpsc::sync_channel(1);
        let handle = connect(&sock, wake_tx);

        let statuses = ask_until_answered(&handle).expect("the daemon answered");
        assert_eq!(statuses[&PathBuf::from("/wt/a")].lines_added, 7);
        assert_eq!(
            statuses[&PathBuf::from("/wt/a")].head_oid.as_deref(),
            Some("head-oid"),
            "the reply keeps the fingerprint the snapshot path strips"
        );
        assert!(
            handle.take().is_some(),
            "the pushed snapshot still reached the snapshot handler"
        );
    }

    #[test]
    fn a_request_without_a_daemon_gives_up_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let (wake_tx, _wake_rx) = mpsc::sync_channel(1);
        let handle = connect(&dir.path().join("absent.sock"), wake_tx);

        let started = Instant::now();
        assert!(
            handle
                .request_git_statuses(vec![PathBuf::from("/wt/a")], false, Duration::from_secs(30),)
                .is_none()
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "an absent daemon must not cost the caller the timeout"
        );
    }

    #[test]
    fn a_daemon_that_exits_mid_session_stops_answering() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        // One answer, then the fake daemon closes the connection and its
        // listener; removing the socket leaves nothing to reconnect to.
        fake_daemon(UnixListener::bind(&sock).unwrap(), 1);

        let (wake_tx, _wake_rx) = mpsc::sync_channel(1);
        let handle = connect(&sock, wake_tx);
        assert!(ask_until_answered(&handle).is_some());
        std::fs::remove_file(&sock).unwrap();

        for _ in 0..200 {
            if handle
                .request_git_statuses(
                    vec![PathBuf::from("/wt/a")],
                    false,
                    Duration::from_millis(100),
                )
                .is_none()
            {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("requests kept being answered after the daemon exited");
    }
}
