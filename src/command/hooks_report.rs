//! `workmux hooks-report` — emit an `agent.session` capability event.
//!
//! Run from the agent's **SessionStart** hook (alongside `workmux signal
//! session-ready`). It records, at the very start of a session, which workmux
//! hooks are actually installed for the agent — so a missing or stale hook setup
//! (the usual cause of "the pipeline never advances": no `turn-done` /
//! `session-ready` hooks) is visible in the event log from the first moment,
//! instead of being inferred later from absent signals.

use anyhow::Result;

use crate::agent::setup::StatusCheck;

/// Emit the capability report for the current pane's agent (Claude — its
/// SessionStart hook is what invokes this). Best-effort and never fatal: a
/// session-start hook must not break the session.
pub fn run(pane: Option<&str>) -> Result<()> {
    let pane = pane
        .map(str::to_string)
        .or_else(|| std::env::var("TMUX_PANE").ok());

    let status =
        crate::agent::setup::claude::check().unwrap_or_else(|e| StatusCheck::Error(e.to_string()));

    let (state, detail): (&str, Vec<String>) = match &status {
        StatusCheck::Installed => ("installed", Vec::new()),
        StatusCheck::Stale { missing } => ("stale", missing.clone()),
        StatusCheck::NotInstalled => ("missing", Vec::new()),
        StatusCheck::Error(e) => ("error", vec![e.clone()]),
    };

    crate::wm_evt!(
        "agent.session",
        agent = "claude",
        pane = ?pane,
        hooks = state,
        missing = ?detail,
        side = "hook",
    );
    Ok(())
}
