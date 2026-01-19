//! Pane handshake mechanisms for shell startup synchronization.
//!
//! Different backends use different mechanisms to ensure a shell is ready
//! before sending commands to a pane.

use anyhow::{Context, Result, anyhow};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, trace, warn};

use crate::cmd::Cmd;

/// Trait for pane handshake mechanisms.
///
/// A handshake ensures the shell has started in a pane before sending commands.
/// Different backends may use different mechanisms (tmux wait-for, named pipes, etc.)
pub trait PaneHandshake: Send {
    /// Returns the command that wraps the shell to signal readiness.
    /// This is formatted for shell evaluation (e.g., passing to tmux).
    fn wrapper_command(&self, shell: &str) -> String;

    /// Waits for the handshake signal, consuming the handshake object.
    fn wait(self: Box<Self>) -> Result<()>;
}

/// Timeout for waiting for pane readiness (seconds)
const HANDSHAKE_TIMEOUT_SECS: u64 = 5;

/// Manages the tmux wait-for handshake protocol for pane synchronization.
///
/// This struct encapsulates the channel-based handshake mechanism that ensures
/// the shell is ready before sending commands. The handshake uses tmux's `wait-for`
/// feature with channel locking to synchronize between the process spawning the
/// pane and the shell that starts inside it.
///
/// # Protocol
/// 1. Lock a unique channel (on construction)
/// 2. Start the shell with a wrapper that unlocks the channel when ready
/// 3. Wait for the shell to signal readiness (wait blocks until unlock)
/// 4. Clean up the channel
pub struct TmuxHandshake {
    channel: String,
}

impl TmuxHandshake {
    /// Create a new handshake and lock the channel.
    ///
    /// The channel must be locked before spawning the pane to ensure we don't
    /// miss the signal even if the shell starts instantly.
    pub fn new() -> Result<Self> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let pid = std::process::id();
        let channel = format!("wm_ready_{}_{}", pid, nanos);

        // Lock the channel (ensures we don't miss the signal)
        Cmd::new("tmux")
            .args(&["wait-for", "-L", &channel])
            .run()
            .context("Failed to initialize wait channel")?;

        Ok(Self { channel })
    }
}

impl PaneHandshake for TmuxHandshake {
    /// Build a shell wrapper command that signals readiness.
    ///
    /// The wrapper briefly disables echo while signaling the channel, restores it,
    /// then exec's into the shell so the TTY starts in a normal state.
    ///
    /// We wrap in `sh -c "..."` with double quotes to ensure the command works when
    /// tmux's default-shell is a non-POSIX shell like nushell. Single-quote escaping
    /// (`'\''`) doesn't work reliably when nushell parses the command before passing
    /// it to sh.
    fn wrapper_command(&self, shell: &str) -> String {
        // Two-step escaping for the shell path:
        // 1. Escape for inner single-quoted context (exec '...')
        let inner_shell = shell.replace('\'', "'\\''");
        // 2. Escape for outer double-quoted context (sh -c "...")
        let escaped_shell = inner_shell
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`");
        format!(
            "sh -c \"stty -echo 2>/dev/null; tmux wait-for -U {}; stty echo 2>/dev/null; exec '{}' -l\"",
            self.channel, escaped_shell
        )
    }

    /// Wait for the shell to signal it is ready, then clean up.
    ///
    /// This method consumes the handshake to ensure cleanup happens exactly once.
    /// Uses a polling loop with timeout to prevent indefinite hangs if the pane
    /// fails to start.
    fn wait(self: Box<Self>) -> Result<()> {
        debug!(channel = %self.channel, "tmux:handshake start");

        let mut child = std::process::Command::new("tmux")
            .args(["wait-for", "-L", &self.channel])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Failed to spawn tmux wait-for command")?;

        let start = Instant::now();
        let timeout = Duration::from_secs(HANDSHAKE_TIMEOUT_SECS);

        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        // Cleanup: unlock the channel we just re-locked
                        Cmd::new("tmux")
                            .args(&["wait-for", "-U", &self.channel])
                            .run()
                            .context("Failed to cleanup wait channel")?;
                        debug!(channel = %self.channel, "tmux:handshake success");
                        return Ok(());
                    } else {
                        // Attempt cleanup even on failure
                        let _ = Cmd::new("tmux")
                            .args(&["wait-for", "-U", &self.channel])
                            .run();
                        warn!(channel = %self.channel, status = ?status.code(), "tmux:handshake failed (wait-for error)");
                        return Err(anyhow!(
                            "Pane handshake failed - tmux wait-for returned error"
                        ));
                    }
                }
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait(); // Ensure process is reaped

                        // Attempt cleanup
                        let _ = Cmd::new("tmux")
                            .args(&["wait-for", "-U", &self.channel])
                            .run();

                        warn!(
                            channel = %self.channel,
                            timeout_secs = HANDSHAKE_TIMEOUT_SECS,
                            "tmux:handshake timeout"
                        );
                        return Err(anyhow!(
                            "Pane handshake timed out after {}s - shell may have failed to start",
                            HANDSHAKE_TIMEOUT_SECS
                        ));
                    }
                    trace!(
                        channel = %self.channel,
                        elapsed_ms = start.elapsed().as_millis(),
                        "tmux:handshake waiting"
                    );
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = Cmd::new("tmux")
                        .args(&["wait-for", "-U", &self.channel])
                        .run();
                    warn!(channel = %self.channel, error = %e, "tmux:handshake error");
                    return Err(anyhow!("Error waiting for pane handshake: {}", e));
                }
            }
        }
    }
}
