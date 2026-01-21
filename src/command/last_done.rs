use anyhow::Result;
use tracing::debug;

use crate::multiplexer::{AgentStatus, BackendType, create_backend};
use crate::state::StateStore;
use crate::tmux;

/// Switch to the agent that most recently completed its task.
///
/// Uses a persistent stack of done panes stored in a tmux server variable
/// for fast lookups. Cycles through completed agents on repeated invocations.
/// Falls back to StateStore-based lookup if the tmux stack is empty.
pub fn run() -> Result<()> {
    // First try the fast tmux done stack
    if tmux::switch_to_last_completed()? {
        return Ok(());
    }

    // Fall back to StateStore-based lookup for cross-session support
    // Skip config loading (which triggers git commands) - just check env var
    let backend_type = match std::env::var("WORKMUX_BACKEND") {
        Ok(s) if s.eq_ignore_ascii_case("wezterm") => BackendType::WezTerm,
        _ => BackendType::Tmux,
    };

    let mux = create_backend(backend_type);
    let store = StateStore::new()?;

    // Read agent state directly from disk without validating against tmux.
    // This avoids O(n) tmux queries. Dead panes are handled during switch.
    let agents = store.list_all_agents()?;

    // Filter to done agents for current backend/instance
    let backend_name = mux.name();
    let instance_id = mux.instance_id();
    let mut done_agents: Vec<_> = agents
        .into_iter()
        .filter(|a| {
            a.status == Some(AgentStatus::Done)
                && a.pane_key.backend == backend_name
                && a.pane_key.instance == instance_id
        })
        .collect();

    debug!(count = done_agents.len(), "done agents");

    if done_agents.is_empty() {
        println!("No completed agents found");
        return Ok(());
    }

    // Sort by timestamp descending (most recent first)
    done_agents.sort_by(|a, b| b.status_ts.cmp(&a.status_ts));

    // Get current pane to determine where we are in the cycle
    let current_pane = mux.current_pane_id();
    let current_idx = current_pane.as_ref().and_then(|current| {
        done_agents
            .iter()
            .position(|a| &a.pane_key.pane_id == current)
    });

    let start_idx = match current_idx {
        Some(idx) => (idx + 1) % done_agents.len(),
        None => 0,
    };

    // Try to switch, skipping dead panes
    for i in 0..done_agents.len() {
        let idx = (start_idx + i) % done_agents.len();
        let pane_id = &done_agents[idx].pane_key.pane_id;

        if mux.switch_to_pane(pane_id).is_ok() {
            return Ok(());
        }
        debug!(pane_id, "pane dead, trying next");
    }

    println!("No active completed agents found");
    Ok(())
}
