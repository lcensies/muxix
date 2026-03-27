//! Unix socket client for receiving snapshots from the sidebar daemon.

use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use super::snapshot::SidebarSnapshot;

type Latest = Arc<Mutex<Option<SidebarSnapshot>>>;

/// Receives snapshots from the daemon over a Unix socket.
pub struct SnapshotReceiver {
    latest: Latest,
}

impl SnapshotReceiver {
    /// Connect to daemon socket with retry. Spawns background reader thread.
    pub fn connect(socket_path: &Path) -> Self {
        let latest: Latest = Arc::new(Mutex::new(None));
        let latest_clone = latest.clone();
        let path = socket_path.to_path_buf();

        thread::spawn(move || {
            Self::connection_loop(&path, &latest_clone);
        });

        Self { latest }
    }

    /// Take the latest snapshot (if any). Returns None if no new data since last call.
    pub fn take(&self) -> Option<SidebarSnapshot> {
        self.latest.lock().unwrap().take()
    }

    fn connection_loop(path: &Path, latest: &Latest) {
        let mut backoff = Duration::from_millis(50);
        let max_backoff = Duration::from_secs(2);

        loop {
            if let Ok(stream) = UnixStream::connect(path) {
                backoff = Duration::from_millis(50);
                Self::read_loop(stream, latest);
                // read_loop returned = disconnected, retry
            }
            thread::sleep(backoff);
            backoff = (backoff * 2).min(max_backoff);
        }
    }

    fn read_loop(mut stream: UnixStream, latest: &Latest) {
        const MAX_PAYLOAD: usize = 1024 * 1024; // 1MB sanity limit
        loop {
            let mut len_buf = [0u8; 4];
            if stream.read_exact(&mut len_buf).is_err() {
                break;
            }
            let len = u32::from_be_bytes(len_buf) as usize;
            if len > MAX_PAYLOAD {
                break; // Corrupt stream, reconnect
            }

            let mut buf = vec![0u8; len];
            if stream.read_exact(&mut buf).is_err() {
                break;
            }

            if let Ok(snapshot) = serde_json::from_slice::<SidebarSnapshot>(&buf) {
                // Always overwrite with latest
                *latest.lock().unwrap() = Some(snapshot);
            }
        }
    }
}
