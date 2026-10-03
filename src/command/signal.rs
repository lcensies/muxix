//! `muxix signal <kind>` — out-of-band agent signalling.
//!
//! Three modes:
//! 1. PANE-KEYED TURN SIGNALS: keyed by $TMUX_PANE, for agent lifecycle hooks
//! 2. NODE-KEYED AGENT SIGNALS: keyed by --node, for inter-stage messaging
//! 3. PANE-KEYED COMPLETION SIGNALS: `done`/`error` without --node, for
//!    agent-authored task completion (see `AgentState.completion`)
//!
//! See [`crate::signals::turn`], [`crate::signals`], and
//! [`crate::state::persist_agent_completion`] for file layouts / storage.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};

use crate::multiplexer::{create_backend, detect_backend};
use crate::signals;
use crate::signals::turn; // For hook_signal path generation
use crate::state::{self, Completion, CompletionKind};

/// Write signal for `kind` against the current pane (turn-based or
/// completion-based) or node (agent-based).
///
/// PANE-KEYED TURN KINDS (uses $TMUX_PANE or --pane):
/// - `turn-done`   — agent finished a response turn (Stop hook).
/// - `needs-input` — agent is blocked on a user question (Notification hook).
/// - `working`     — agent resumed; clears `needs-input` (PostToolUse / UserPromptSubmit).
/// - `proceed`     — release the current gate node and continue (e.g. `/implement`).
/// - `reject`      — release the current gate node with rejection + optional `feedback`.
///
/// `done`/`error` are dual-mode:
/// - With `--node`: NODE-KEYED, writes the pipeline hook-signal file for inter-stage messaging.
/// - Without `--node`: PANE-KEYED, writes an agent-authored `Completion` onto
///   `AgentState` (uses $TMUX_PANE or --pane) — this is the signal `wait`/`status` read.
pub fn run(
    kind: &str,
    pane: Option<&str>,
    node: Option<&str>,
    feedback: Option<&str>,
) -> Result<()> {
    match kind {
        // PANE-KEYED TURN SIGNALS (agent lifecycle hooks)
        "turn-done" | "needs-input" | "working" | "proceed" | "reject" | "session-ready" => {
            run_pane_signal(kind, pane, feedback)
        }
        // done/error: node-keyed if --node given, else pane-keyed completion
        "done" | "error" if node.is_some() => run_node_signal(kind, node, feedback),
        "done" | "error" => run_completion_signal(kind, pane, feedback),
        other => Err(anyhow!("unknown signal kind {other:?}")),
    }
}

/// Handle pane-keyed signals (turn-done, proceed, reject, etc.)
fn run_pane_signal(kind: &str, pane: Option<&str>, feedback: Option<&str>) -> Result<()> {
    let pane = pane
        .map(str::to_string)
        .or_else(turn::current_pane)
        .ok_or_else(|| anyhow!("no pane id available (set $TMUX_PANE or pass --pane)"))?;

    match kind {
        "turn-done" => {
            fs::write(turn::turn_done_path(&pane), b"")?;
        }
        "session-ready" => {
            fs::write(turn::session_ready_path(&pane), b"")?;
        }
        "needs-input" => {
            fs::write(turn::needs_input_path(&pane), b"")?;
        }
        "working" => {
            let _ = fs::remove_file(turn::needs_input_path(&pane));
        }
        "proceed" | "reject" => {
            let payload = serde_json::json!({
                "approved": kind == "proceed",
                "feedback": feedback,
            });
            fs::write(turn::proceed_path(&pane), payload.to_string())?;
        }
        _ => unreachable!(),
    }
    // Records the write from the *agent hook* process. Same `muxix.log` as the
    // runner's observe events, so write→observe latency is directly measurable.
    crate::wm_evt!("signal.write", kind = kind, pane = %pane, side = "hook");
    Ok(())
}

/// Handle pane-keyed completion signals (done/error without --node).
///
/// Writes an agent-authored `Completion` onto `AgentState` for the resolved
/// pane. This is the signal `wait --status completed|failed` and `status`
/// read (see design D1); distinct from the turn-based `AgentStatus`.
fn run_completion_signal(kind: &str, pane: Option<&str>, feedback: Option<&str>) -> Result<()> {
    let pane = pane
        .map(str::to_string)
        .or_else(turn::current_pane)
        .ok_or_else(|| {
            anyhow!("no pane id available for {kind} signal (set $TMUX_PANE, pass --pane, or pass --node)")
        })?;

    let completion_kind = match kind {
        "done" => CompletionKind::Completed,
        "error" => CompletionKind::Failed,
        _ => unreachable!(),
    };
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mux = create_backend(detect_backend());
    state::persist_agent_completion(
        mux.as_ref(),
        &pane,
        Some(Completion {
            kind: completion_kind,
            feedback: feedback.map(str::to_string),
            ts,
        }),
    );
    crate::wm_evt!("signal.write", kind = kind, pane = %pane, side = "agent");
    println!("Signal {kind} written for pane {pane}");
    Ok(())
}

/// Handle node-keyed signals (done, error) for inter-stage messaging.
/// Writes to the hook signal file that the runner polls.
fn run_node_signal(kind: &str, node: Option<&str>, feedback: Option<&str>) -> Result<()> {
    let node_id = node.ok_or_else(|| anyhow!("--node is required for {kind} signal"))?;
    let task_id = std::env::var("TASK_ID").unwrap_or_else(|_| "unknown".to_string());

    // Determine the stop reason based on kind and feedback
    let stop_reason = match kind {
        "done" => "explicit_done".to_string(),
        "error" => "error".to_string(),
        _ => unreachable!(),
    };

    let payload = serde_json::json!({
        "stop_reason": stop_reason,
        "feedback": feedback,
        "turn_count": 0,
        "idle_turns": 0,
    });

    let signal_path = signals::paths::hook_signal(&task_id, node_id);
    fs::write(&signal_path, payload.to_string())?;
    crate::wm_evt!("signal.write", kind = kind, node = node_id, side = "agent");
    println!(
        "Signal {} written for node {} ({})",
        kind,
        node_id,
        signal_path.display()
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn proceed_writes_approved_signal() {
        let pane = "%wmtest-proceed-9001";
        let path = turn::proceed_path(pane);
        let _ = fs::remove_file(&path);
        run("proceed", Some(pane), None, None).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["approved"], Value::Bool(true));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn reject_writes_feedback_signal() {
        let pane = "%wmtest-reject-9002";
        let path = turn::proceed_path(pane);
        let _ = fs::remove_file(&path);
        run("reject", Some(pane), None, Some("needs tests")).unwrap();
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["approved"], Value::Bool(false));
        assert_eq!(v["feedback"], Value::String("needs tests".into()));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn unknown_kind_errors() {
        assert!(run("bogus", Some("%wmtest-x"), None, None).is_err());
    }

    #[test]
    fn agent_signal_without_pane_or_node_errors() {
        // Neither --pane nor --node nor $TMUX_PANE available: must error, not
        // silently fall back to a wrong pane.
        unsafe {
            std::env::remove_var("TMUX_PANE");
        }
        assert!(run("done", None, None, None).is_err());
        assert!(run("error", None, None, None).is_err());
    }

    #[test]
    fn done_signal_creates_file() {
        let node_id = "done-test-node";
        let task_id = "done-task-id";
        let path = signals::paths::hook_signal(task_id, node_id);
        let _ = fs::remove_file(&path);

        // Set TASK_ID so run() uses it
        unsafe {
            std::env::set_var("TASK_ID", task_id);
        }

        run("done", None, Some(node_id), Some("tests passed")).unwrap();
        assert!(path.exists(), "Signal file should be created");

        // Verify it contains valid JSON
        let content = fs::read_to_string(&path).expect("Should read signal file");
        let v: Value = serde_json::from_str(&content).expect("Should parse JSON");
        assert_eq!(v["stop_reason"], "explicit_done");
        assert_eq!(v["feedback"], "tests passed");

        let _ = fs::remove_file(&path);
    }
}
