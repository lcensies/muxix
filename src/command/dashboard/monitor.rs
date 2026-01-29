//! Agent stall detection for the dashboard.
//!
//! Monitors working agents to detect when they've stalled (no pane content changes).

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::multiplexer::{AgentPane, AgentStatus, Multiplexer};
use crate::state::{PaneKey, StateStore};

/// Monitors agent panes to detect stalls and interrupts.
/// Tracks pane content hashes to identify when a "working" agent has stopped producing output.
pub struct AgentMonitor {
    /// Tracks pane content hashes for detecting stalled working agents
    content_hashes: HashMap<String, u64>,
    /// Set of pane IDs that have been detected as stalled (to avoid re-checking)
    stalled_panes: HashSet<String>,
}

impl AgentMonitor {
    pub fn new() -> Self {
        Self {
            content_hashes: HashMap::new(),
            stalled_panes: HashSet::new(),
        }
    }

    /// Check if a working agent pane is stalled (content unchanged).
    /// Returns true if the agent was detected as stalled.
    fn check_if_stalled(&mut self, pane_id: &str, mux: &dyn Multiplexer) -> bool {
        let Some(content) = mux.capture_pane(pane_id, 50) else {
            return false;
        };

        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        content.hash(&mut hasher);
        let current_hash = hasher.finish();

        let pane_key = pane_id.to_string();
        if let Some(&prev_hash) = self.content_hashes.get(&pane_key) {
            if prev_hash == current_hash {
                // Content unchanged - agent is stalled
                self.stalled_panes.insert(pane_key.clone());
                self.content_hashes.remove(&pane_key);
                return true;
            } else {
                // Content changed - agent is still working
                self.content_hashes.insert(pane_key, current_hash);
            }
        } else {
            // First time seeing this pane, start tracking
            self.content_hashes.insert(pane_key, current_hash);
        }
        false
    }

    /// Clean up tracking data for panes that are no longer active.
    fn cleanup_cache(&mut self, agents: &[AgentPane]) {
        // Remove hashes for panes that are no longer working
        let working_pane_ids: HashSet<_> = agents
            .iter()
            .filter(|a| matches!(a.status, Some(AgentStatus::Working)))
            .map(|a| a.pane_id.as_str())
            .collect();
        self.content_hashes
            .retain(|k, _| working_pane_ids.contains(k.as_str()));

        // Remove stalled panes that are no longer in the agent list or no longer have working status
        let all_pane_ids: HashSet<_> = agents.iter().map(|a| a.pane_id.as_str()).collect();
        self.stalled_panes.retain(|pane_id| {
            all_pane_ids.contains(pane_id.as_str())
                && agents
                    .iter()
                    .find(|a| &a.pane_id == pane_id)
                    .map(|a| matches!(a.status, Some(AgentStatus::Working)))
                    .unwrap_or(false)
        });
    }

    /// Process agents to detect stalls. Modifies agent status in place for stalled agents.
    pub fn process_stalls(
        &mut self,
        mut agents: Vec<AgentPane>,
        _working_icon: &str,
        mux: &dyn Multiplexer,
    ) -> Vec<AgentPane> {
        // Check each working agent for stalls
        for agent in &mut agents {
            if matches!(agent.status, Some(AgentStatus::Working))
                && !self.stalled_panes.contains(&agent.pane_id)
                && self.check_if_stalled(&agent.pane_id, mux)
            {
                // Mark as interrupted in state store and multiplexer
                mark_agent_interrupted(&agent.pane_id, mux);
                // Clear status in the agent object to reflect the interrupt
                agent.status = None;
                // Update timestamp to current time (elapsed time starts from 0)
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                agent.status_ts = Some(now);
            }
        }

        // Clean up tracking data
        self.cleanup_cache(&agents);

        agents
    }
}

impl Default for AgentMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Mark an agent as interrupted by clearing its status and resetting timestamps.
/// Used when an agent is detected as stalled (no pane content changes).
/// Updates both StateStore and multiplexer window status.
fn mark_agent_interrupted(pane_id: &str, mux: &dyn Multiplexer) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // Clear window-level status icon
    let _ = mux.clear_status(pane_id);

    // Update state in StateStore
    if let Ok(store) = StateStore::new() {
        // Get live pane info to construct the PaneKey
        if let Ok(Some(_live)) = mux.get_live_pane_info(pane_id) {
            let pane_key = PaneKey {
                backend: mux.name().to_string(),
                instance: mux.instance_id(),
                pane_id: pane_id.to_string(),
            };

            // Load existing state, update it, and save
            if let Ok(agents) = store.list_all_agents()
                && let Some(mut state) = agents.into_iter().find(|a| a.pane_key == pane_key)
            {
                // Clear status and reset timestamp
                state.status = None;
                state.status_ts = Some(now);
                state.updated_ts = now;
                let _ = store.upsert_agent(&state);
            }
        }
    }
}
