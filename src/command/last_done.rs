use std::cmp::Reverse;

use anyhow::Result;
use tracing::debug;

use crate::multiplexer::{AgentStatus, create_backend, detect_backend};
use crate::state::{AgentState, StateStore};

/// Switch to the agent that most recently completed or is waiting for input.
///
/// Finds all agents with "done" or "waiting" status from the StateStore and
/// switches to the one with the most recent timestamp. Cycles through matching
/// agents on repeated invocations.
pub fn run() -> Result<()> {
    let mux = create_backend(detect_backend());
    let store = StateStore::new()?;

    // Read agent state directly from disk without validating against tmux.
    // This avoids O(n) tmux queries. Dead panes are handled during switch.
    let agents = store.list_all_agents()?;

    // Filter to done/waiting agents for current backend/instance
    let backend_name = mux.name();
    let instance_id = mux.instance_id();
    let mut done_agents: Vec<_> = agents
        .into_iter()
        .filter(|a| {
            matches!(
                a.status,
                Some(AgentStatus::Done) | Some(AgentStatus::Waiting)
            ) && a.pane_key.backend == backend_name
                && a.pane_key.instance == instance_id
        })
        .collect();

    debug!(count = done_agents.len(), "done/waiting agents");

    if done_agents.is_empty() {
        println!("No completed or waiting agents found");
        return Ok(());
    }

    // Sort by status_ts descending (most recent first), with updated_ts as
    // tiebreaker for agents that changed status in the same second.
    sort_by_recency(&mut done_agents);

    // Get current pane to determine where we are in the cycle
    // Use active_pane_id() instead of current_pane_id() - env var is stale in run-shell
    let current_pane = mux.active_pane_id();
    debug!(current_pane = ?current_pane, "active pane");

    let current_idx = current_pane.as_ref().and_then(|current| {
        done_agents
            .iter()
            .position(|a| &a.pane_key.pane_id == current)
    });

    let target_idx = pick_target(current_idx, done_agents.len());
    debug!(current_idx = ?current_idx, target_idx, "cycle position");

    // Try to switch, skipping dead panes
    for i in 0..done_agents.len() {
        let idx = (target_idx + i) % done_agents.len();
        let agent = &done_agents[idx];
        let pane_id = &agent.pane_key.pane_id;
        let window_hint = agent.window_name.as_deref();

        debug!(
            pane_id,
            status = ?agent.status,
            status_ts = ?agent.status_ts,
            "trying agent"
        );

        if let Err(e) = mux.switch_to_pane(pane_id, window_hint) {
            debug!(pane_id, error = %e, "pane dead, trying next");
        } else {
            return Ok(());
        }
    }

    println!("No active completed or waiting agents found");
    Ok(())
}

/// Sort agents by recency: most recent status change first, with updated_ts
/// as tiebreaker for deterministic ordering.
fn sort_by_recency(agents: &mut [AgentState]) {
    agents.sort_by_key(|a| {
        (
            Reverse(a.status_ts),
            Reverse(a.updated_ts),
            Reverse(a.pane_key.pane_id.clone()),
        )
    });
}

/// Pick the target index in the sorted list.
/// If currently on a done/waiting agent, advance to the next one (cycling).
/// Otherwise, start at the most recent (index 0).
fn pick_target(current_idx: Option<usize>, len: usize) -> usize {
    match current_idx {
        Some(idx) => (idx + 1) % len,
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PaneKey;
    use std::path::PathBuf;

    fn make_agent(
        pane_id: &str,
        status: AgentStatus,
        status_ts: u64,
        updated_ts: u64,
    ) -> AgentState {
        AgentState {
            pane_key: PaneKey {
                backend: "tmux".to_string(),
                instance: "default".to_string(),
                pane_id: pane_id.to_string(),
            },
            workdir: PathBuf::from("/tmp"),
            status: Some(status),
            status_ts: Some(status_ts),
            pane_title: None,
            pane_pid: 1000,
            command: "node".to_string(),
            updated_ts,
            window_name: Some("wm-test".to_string()),
            session_name: Some("main".to_string()),
            boot_id: None,
        }
    }

    #[test]
    fn test_sort_by_recency_orders_most_recent_first() {
        let mut agents = vec![
            make_agent("%1", AgentStatus::Done, 100, 100),
            make_agent("%2", AgentStatus::Done, 300, 300),
            make_agent("%3", AgentStatus::Done, 200, 200),
        ];
        sort_by_recency(&mut agents);
        assert_eq!(agents[0].pane_key.pane_id, "%2");
        assert_eq!(agents[1].pane_key.pane_id, "%3");
        assert_eq!(agents[2].pane_key.pane_id, "%1");
    }

    #[test]
    fn test_sort_uses_updated_ts_as_tiebreaker() {
        let mut agents = vec![
            make_agent("%1", AgentStatus::Done, 100, 200),
            make_agent("%2", AgentStatus::Done, 100, 300),
        ];
        sort_by_recency(&mut agents);
        // Same status_ts, but %2 has more recent updated_ts
        assert_eq!(agents[0].pane_key.pane_id, "%2");
        assert_eq!(agents[1].pane_key.pane_id, "%1");
    }

    #[test]
    fn test_sort_none_status_ts_goes_last() {
        let mut agents = vec![
            AgentState {
                status_ts: None,
                ..make_agent("%1", AgentStatus::Done, 0, 100)
            },
            make_agent("%2", AgentStatus::Done, 50, 50),
        ];
        sort_by_recency(&mut agents);
        assert_eq!(agents[0].pane_key.pane_id, "%2");
        assert_eq!(agents[1].pane_key.pane_id, "%1");
    }

    #[test]
    fn test_sort_mixed_done_and_waiting() {
        let mut agents = vec![
            make_agent("%1", AgentStatus::Done, 100, 100),
            make_agent("%2", AgentStatus::Waiting, 200, 200),
            make_agent("%3", AgentStatus::Done, 300, 300),
        ];
        sort_by_recency(&mut agents);
        // Pure recency order regardless of status type
        assert_eq!(agents[0].pane_key.pane_id, "%3"); // Done, most recent
        assert_eq!(agents[1].pane_key.pane_id, "%2"); // Waiting
        assert_eq!(agents[2].pane_key.pane_id, "%1"); // Done, oldest
    }

    #[test]
    fn test_pick_target_not_on_done_agent() {
        assert_eq!(pick_target(None, 3), 0);
    }

    #[test]
    fn test_pick_target_cycles_to_next() {
        assert_eq!(pick_target(Some(0), 3), 1);
        assert_eq!(pick_target(Some(1), 3), 2);
    }

    #[test]
    fn test_pick_target_wraps_around() {
        assert_eq!(pick_target(Some(2), 3), 0);
    }

    #[test]
    fn test_pick_target_single_agent() {
        assert_eq!(pick_target(Some(0), 1), 0);
    }
}
