//! Out-of-band agent turn signalling.
//!
//! Replaces the old in-band completion contracts (asking the agent to `touch` a
//! file, or scraping its stdout for a sentinel) with harness-driven markers that
//! work regardless of agent mode — including Claude Code plan mode, where the
//! agent cannot run shell commands.
//!
//! Files are keyed by **tmux pane id** rather than node/task id: the runner
//! already knows the pane it drives (`target_pane`), and an agent's hook
//! subprocess inherits `$TMUX_PANE`, so both sides agree on the key with no extra
//! environment plumbing.
//!
//! - `turn-done`   — written by the agent's **Stop** hook when a response turn
//!   ends. The runner clears it before sending a prompt and polls for it.
//! - `needs-input` — written by the agent's **Notification** hook
//!   (`permission_prompt|elicitation_dialog`) when the agent blocks on a question
//!   to the user, and removed by the **UserPromptSubmit/PostToolUse** hooks when
//!   the agent resumes. While present, the runner pauses the node's timeout.

use std::path::PathBuf;

/// Sanitise a tmux pane id (`%5`, `sess:1.2`) into a filename-safe token.
/// Mirrors the sanitisation the runner has always used for pane-scoped temp files.
fn sanitize(pane: &str) -> String {
    pane.trim_start_matches('%').replace(':', "-")
}

/// Marker written by the agent's Stop hook when a response turn completes.
pub fn turn_done_path(pane: &str) -> PathBuf {
    std::env::temp_dir().join(format!("workmux-turn-{}.done", sanitize(pane)))
}

/// Marker written by the agent's **SessionStart** hook when a (re)launched agent
/// session has initialised in the pane. The runner clears it just before a pane
/// respawn and waits for the fresh one as the readiness gate — a deterministic
/// "the new post-respawn agent is up" signal that no content scrape can give.
pub fn session_ready_path(pane: &str) -> PathBuf {
    std::env::temp_dir().join(format!("workmux-session-ready-{}", sanitize(pane)))
}

/// Marker written while the agent is blocked on a question to the user.
pub fn needs_input_path(pane: &str) -> PathBuf {
    std::env::temp_dir().join(format!("workmux-needs-input-{}", sanitize(pane)))
}

/// Pane-keyed approval signal, written by `workmux signal proceed|reject` (from the
/// agent pane, e.g. the `/implement` slash command). Lets the human (or the agent
/// itself) release the current gate node without knowing its node id — the runner
/// already knows the pane it drives. Carries the same `{approved, feedback}` JSON
/// shape as the node-keyed breakpoint signal, so `wait_for_approval` parses both.
pub fn proceed_path(pane: &str) -> PathBuf {
    std::env::temp_dir().join(format!("workmux-proceed-{}.json", sanitize(pane)))
}

/// Resolve the current pane id from the environment (set by tmux for any process
/// running inside a pane, and inherited by agent hook subprocesses).
pub fn current_pane() -> Option<String> {
    std::env::var("TMUX_PANE").ok().filter(|s| !s.is_empty())
}
