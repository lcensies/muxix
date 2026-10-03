//! Signal abstraction for node state transitions.
//!
//! Unified message-passing mechanism for different pipeline boundaries:
//! - Hook signals: automatic detection via Stop hook + stop reason routing (impl→test, test→merge)
//! - Agent tools: programmatic signals from agents (feedback between stages)

pub mod event;
pub mod hook;
pub mod turn;
pub mod types;

pub use types::*;

use anyhow::Result;

/// Abstract signal that waits for a state transition (approval, rejection, or auto-advance).
pub trait Signal: Send + Sync {
    /// Poll for a signal. Returns Some(SignalResult) if signal fired, None if still waiting.
    fn check(&self) -> Result<Option<SignalResult>>;

    /// Clear this signal's state (remove temp files, reset markers).
    fn clear(&self) -> Result<()>;

    /// Human-readable name for logging.
    fn name(&self) -> &str;
}

/// Temporary file paths for inter-component signalling.
pub mod paths {
    use std::path::PathBuf;

    fn sanitize(s: &str) -> String {
        s.trim_start_matches('%').replace(':', "-")
    }

    /// Node-keyed breakpoint signal: explicit human approval.
    /// Written by TUI ([a]/[r]), GUI (RPC), or slash commands.
    pub fn node_breakpoint_signal(task_id: &str, node_id: &str) -> PathBuf {
        std::env::temp_dir().join(format!("workmux-bp-{}-{}.json", task_id, node_id))
    }

    /// Pane-keyed approval signal: written by agent or `workmux signal` from the pane.
    /// Lets the agent release a gate node without knowing its node id.
    pub fn pane_proceed_signal(pane: &str) -> PathBuf {
        std::env::temp_dir().join(format!("workmux-proceed-{}.json", sanitize(pane)))
    }

    /// Agent's Stop hook marker: written when a turn ends.
    pub fn turn_done_marker(pane: &str) -> PathBuf {
        std::env::temp_dir().join(format!("workmux-turn-{}.done", sanitize(pane)))
    }

    /// Hook signal: agent has explicitly marked this node as done.
    /// Written by agent tool calls (e.g., `workmux signal done --stage implementation`).
    pub fn hook_signal(task_id: &str, node_id: &str) -> PathBuf {
        std::env::temp_dir().join(format!("workmux-signal-{}-{}.json", task_id, node_id))
    }
}
