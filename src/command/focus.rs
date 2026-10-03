//! `muxix focus <agent-id | name>` — switch multiplexer focus to an agent pane.
//!
//! Resolution order:
//!   1. Exact `agent_id` UUID match (stable, globally unique)
//!   2. Window-name substring match (convenience shorthand)
//!
//! Using the UUID avoids the ambiguity of pane IDs which can be recycled
//! by the multiplexer or shared across different tmux server sockets.

use anyhow::{Context, Result, bail};
use tracing::{debug, warn};

use crate::multiplexer::{Multiplexer, create_backend, detect_backend};
use crate::state::{AgentState, StateStore};

pub fn run(target: &str) -> Result<()> {
    let mux = create_backend(detect_backend());
    let store = StateStore::new()?;

    let agents = store.list_all_agents()?;

    // Before leaving the current agent, checkpoint it if it is a sandboxed
    // agent with checkpointing enabled. This is what lets us switch between
    // (e.g.) opencode TUIs without losing in-flight VM state.
    checkpoint_current_agent(&*mux, &store, &agents);

    // 1. Exact agent_id match (full UUID or unambiguous prefix).
    let by_id: Vec<_> = agents
        .iter()
        .filter(|a| a.agent_id == target || a.agent_id.starts_with(target))
        .collect();

    if by_id.len() == 1 {
        return switch_to(&*mux, by_id[0]);
    }
    if by_id.len() > 1 {
        bail!(
            "agent ID prefix '{}' is ambiguous — use more characters",
            target
        );
    }

    // 2. Window-name substring fallback.
    let matches: Vec<_> = agents
        .iter()
        .filter(|a| {
            a.window_name
                .as_deref()
                .map(|w| w.contains(target))
                .unwrap_or(false)
        })
        .collect();

    match matches.len() {
        0 => bail!(
            "no agent found for '{}' (tried agent ID and window name substring)",
            target
        ),
        1 => switch_to(&*mux, matches[0]),
        _ => {
            // Multiple matches — show a list and let the user pick by agent ID.
            eprintln!("Multiple agents match '{}'. Specify an agent ID:", target);
            for a in &matches {
                eprintln!(
                    "  {} — {} ({})",
                    &a.agent_id[..a.agent_id.len().min(8)],
                    a.window_name.as_deref().unwrap_or("?"),
                    a.workdir.display()
                );
            }
            bail!("ambiguous target — use an agent ID instead");
        }
    }
}

/// Checkpoint the agent occupying the currently-active pane, if it is a
/// sandboxed agent whose project has checkpointing enabled.
///
/// Best-effort: any failure (no active pane, no matching agent, no sandbox,
/// checkpoint error) is logged and otherwise ignored — switching focus must
/// never be blocked by a checkpoint hiccup.
fn checkpoint_current_agent(mux: &dyn Multiplexer, store: &StateStore, agents: &[AgentState]) {
    let Some(active_pane) = mux.active_pane_id() else {
        return;
    };
    let instance = mux.instance_id();
    let backend = mux.name();

    let Some(current) = agents.iter().find(|a| {
        a.pane_key.pane_id == active_pane
            && a.pane_key.instance == instance
            && a.pane_key.backend == backend
    }) else {
        return;
    };

    // Only sandboxed agents can be checkpointed.
    if current.sandbox_id.is_none() {
        return;
    }

    let config = match crate::config::Config::load_with_location_from(&current.workdir, None) {
        Ok((cfg, _)) => cfg,
        Err(e) => {
            debug!(error = %e, "skip checkpoint-on-switch: config load failed");
            return;
        }
    };

    // Checkpoint-on-switch is the mechanism that lets multiple sandboxed agents
    // share a pane with low memory overhead: the one we're leaving is snapshotted
    // (and can later be resumed) so its VM can be released. This is independent
    // of `strategy` — Manual only disables *background* (periodic / mode-switch)
    // checkpoints of a running agent, not the swap performed when switching away.
    if !config.sandbox.checkpoint.is_enabled() {
        return;
    }

    debug!(agent_id = %current.agent_id, "checkpointing agent before focus switch");
    match crate::sandbox::checkpoint::checkpoint_agent(current, &config.sandbox, store) {
        Ok(path) => debug!(path = %path.display(), "checkpoint-on-switch complete"),
        Err(e) => warn!(error = %e, "checkpoint-on-switch failed (continuing)"),
    }
}

fn switch_to(mux: &dyn crate::multiplexer::Multiplexer, state: &AgentState) -> Result<()> {
    let pane_id = &state.pane_key.pane_id;
    let window_hint = state.window_name.as_deref();

    mux.switch_to_pane(pane_id, window_hint)
        .with_context(|| format!("switch to pane {pane_id}"))?;

    Ok(())
}
