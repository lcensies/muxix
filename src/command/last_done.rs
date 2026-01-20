use anyhow::Result;

use crate::config::Config;
use crate::multiplexer::{AgentStatus, create_backend, detect_backend};
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
    let config = Config::load(None)?;
    let mux = create_backend(detect_backend(&config));
    let store = StateStore::new()?;

    // Get all valid agents for current backend
    let agents = store.load_reconciled_agents(mux.as_ref())?;

    // Filter to done agents and sort by timestamp descending (most recent first)
    let mut done_agents: Vec<_> = agents
        .into_iter()
        .filter(|a| a.status == Some(AgentStatus::Done))
        .collect();

    if done_agents.is_empty() {
        println!("No completed agents found");
        return Ok(());
    }

    done_agents.sort_by(|a, b| b.status_ts.cmp(&a.status_ts));

    // Get current pane to determine where we are in the cycle
    let current_pane = mux.current_pane_id();

    // Find current position in the sorted list
    let current_idx = current_pane
        .as_ref()
        .and_then(|current| done_agents.iter().position(|a| &a.pane_id == current));

    // Determine which pane to switch to
    let target_idx = match current_idx {
        Some(idx) => (idx + 1) % done_agents.len(), // Cycle to next (older) agent
        None => 0,                                  // Start with most recent
    };

    mux.switch_to_pane(&done_agents[target_idx].pane_id)?;
    Ok(())
}
