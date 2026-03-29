//! Unix socket client for receiving snapshots from the sidebar daemon.

use std::io::{Read, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::snapshot::SidebarSnapshot;

fn client_log(msg: &str) {
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
        let line = format!("[{:.3}] CLIENT({}): {}\n", now.as_secs_f64(), pid, msg);
        let _ = f.write_all(line.as_bytes());
    }
}

type Latest = Arc<Mutex<Option<SidebarSnapshot>>>;

/// Handle for taking the latest coalesced snapshot.
pub struct SnapshotHandle {
    latest: Latest,
}

impl SnapshotHandle {
    /// Take the latest snapshot (if any). Returns None if no new data.
    pub fn take(&self) -> Option<SidebarSnapshot> {
        self.latest.lock().unwrap().take()
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
    let path = socket_path.to_path_buf();

    thread::spawn(move || {
        connection_loop(&path, &latest_clone, &wake_tx);
    });

    SnapshotHandle { latest }
}

fn connection_loop(path: &Path, latest: &Latest, wake_tx: &mpsc::SyncSender<()>) {
    let min_backoff = Duration::from_millis(50);
    let max_backoff = Duration::from_secs(2);
    // PID-based jitter to prevent 12 clients from phase-locking reconnects
    let jitter = Duration::from_millis((std::process::id() % 100) as u64);
    let mut backoff = min_backoff;

    client_log("connection_loop started");

    loop {
        let connected_at = Instant::now();

        match UnixStream::connect(path) {
            Ok(stream) => {
                client_log("connected to daemon socket");
                backoff = min_backoff; // Reset on any successful connect
                if read_loop(stream, latest, wake_tx).is_err() {
                    client_log("wake channel closed, exiting");
                    break; // Wake channel closed, main thread exited
                }
                let duration = connected_at.elapsed();
                client_log(&format!(
                    "socket closed after {:.1}s, reconnecting",
                    duration.as_secs_f64()
                ));
                // Only reset backoff if connection was stable (lasted >5s).
                // Short-lived connections (daemon accept-then-close) keep
                // exponential backoff to prevent synchronized churn.
                if duration <= Duration::from_secs(5) {
                    backoff = (backoff * 2).min(max_backoff);
                }
            }
            Err(e) => {
                client_log(&format!(
                    "connect failed: {}, backoff={:?}",
                    e,
                    backoff + jitter
                ));
            }
        }

        thread::sleep(backoff + jitter);
    }
}

/// Returns Err if the wake channel is closed (main thread exited).
fn read_loop(
    mut stream: UnixStream,
    latest: &Latest,
    wake_tx: &mpsc::SyncSender<()>,
) -> Result<(), mpsc::SendError<()>> {
    const MAX_PAYLOAD: usize = 1024 * 1024; // 1MB sanity limit
    let mut msg_count = 0u64;
    loop {
        let mut len_buf = [0u8; 4];
        if let Err(e) = stream.read_exact(&mut len_buf) {
            client_log(&format!(
                "read_loop: read len failed: {} (after {} msgs)",
                e, msg_count
            ));
            return Ok(()); // Socket closed, reconnect
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        if len > MAX_PAYLOAD {
            client_log(&format!("read_loop: payload too large: {} bytes", len));
            return Ok(()); // Corrupt stream, reconnect
        }

        let mut buf = vec![0u8; len];
        if let Err(e) = stream.read_exact(&mut buf) {
            client_log(&format!(
                "read_loop: read payload failed: {} (after {} msgs)",
                e, msg_count
            ));
            return Ok(());
        }

        match serde_json::from_slice::<SidebarSnapshot>(&buf) {
            Ok(snapshot) => {
                msg_count += 1;
                let agents = snapshot.agents.len();
                *latest.lock().unwrap() = Some(snapshot);
                // try_send: if a wake is already pending, skip (coalesces)
                // Full = no-op (wake already queued), Disconnected = main exited
                match wake_tx.try_send(()) {
                    Ok(()) => {
                        client_log(&format!(
                            "read_loop: snapshot #{} agents={} wake=sent",
                            msg_count, agents
                        ));
                    }
                    Err(mpsc::TrySendError::Full(())) => {
                        client_log(&format!(
                            "read_loop: snapshot #{} agents={} wake=coalesced",
                            msg_count, agents
                        ));
                    }
                    Err(mpsc::TrySendError::Disconnected(())) => {
                        client_log(&format!(
                            "read_loop: snapshot #{} agents={} wake=disconnected",
                            msg_count, agents
                        ));
                        return Err(mpsc::SendError(()));
                    }
                }
            }
            Err(e) => {
                client_log(&format!(
                    "read_loop: deserialize failed: {} ({} bytes)",
                    e,
                    buf.len()
                ));
            }
        }
    }
}
