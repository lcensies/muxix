//! Snapshot data types and builder for daemon-to-client communication.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::StatusIcons;
use crate::multiplexer::{AgentPane, AgentStatus};

use super::app::SidebarLayoutMode;

/// A complete sidebar state snapshot, pushed from daemon to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidebarSnapshot {
    pub version: u64,
    pub layout_mode: SidebarLayoutMode,
    pub active_windows: HashSet<(String, String)>,
    #[serde(default)]
    pub active_pane_ids: HashSet<String>,
    pub window_pane_counts: HashMap<String, usize>,
    pub agents: Vec<AgentPane>,
}

/// Build a snapshot from reconciled agents and tmux state.
#[allow(clippy::too_many_arguments)]
pub fn build_snapshot(
    mut agents: Vec<AgentPane>,
    tmux_statuses: &HashMap<String, Option<String>>,
    pane_window_ids: &HashMap<String, String>,
    active_windows: HashSet<(String, String)>,
    active_pane_ids: HashSet<String>,
    window_pane_counts: HashMap<String, usize>,
    layout_mode: SidebarLayoutMode,
    status_icons: &StatusIcons,
    version: u64,
) -> SidebarSnapshot {
    let done_icon = status_icons.done();
    let waiting_icon = status_icons.waiting();

    // Suppress Done/Waiting when tmux's auto-clear hook has already cleared
    for agent in &mut agents {
        if let Some(observed) = tmux_statuses.get(&agent.pane_id) {
            match agent.status {
                Some(AgentStatus::Done) if observed.as_deref() != Some(done_icon) => {
                    agent.status = None;
                }
                Some(AgentStatus::Waiting) if observed.as_deref() != Some(waiting_icon) => {
                    agent.status = None;
                }
                _ => {}
            }
        }
    }

    // Sort by recency
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    agents.sort_by_cached_key(|a| {
        let elapsed = a
            .status_ts
            .map(|ts| now.saturating_sub(ts))
            .unwrap_or(u64::MAX);
        let pane_num: u64 = a
            .pane_id
            .strip_prefix('%')
            .unwrap_or(&a.pane_id)
            .parse()
            .unwrap_or(u64::MAX);
        (elapsed, pane_num)
    });

    // Populate window_id from the tmux state lookup
    for agent in &mut agents {
        if let Some(wid) = pane_window_ids.get(&agent.pane_id) {
            agent.window_id = wid.clone();
        }
    }

    SidebarSnapshot {
        version,
        layout_mode,
        active_windows,
        active_pane_ids,
        window_pane_counts,
        agents,
    }
}
