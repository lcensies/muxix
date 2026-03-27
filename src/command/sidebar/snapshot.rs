//! Snapshot data types and builder for daemon-to-client communication.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
    pub window_pane_counts: HashMap<String, usize>,
    pub agents: Vec<SnapshotAgent>,
}

/// Agent data within a snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotAgent {
    pub pane_id: String,
    pub session: String,
    pub window_name: String,
    pub window_id: String,
    pub path: PathBuf,
    pub pane_title: Option<String>,
    pub status: Option<AgentStatus>,
    pub status_ts: Option<u64>,
}

impl SnapshotAgent {
    /// Convert back to AgentPane for rendering.
    pub fn to_agent_pane(&self) -> AgentPane {
        AgentPane {
            pane_id: self.pane_id.clone(),
            session: self.session.clone(),
            window_name: self.window_name.clone(),
            path: self.path.clone(),
            pane_title: self.pane_title.clone(),
            status: self.status,
            status_ts: self.status_ts,
        }
    }
}

/// Build a snapshot from reconciled agents and tmux state.
#[allow(clippy::too_many_arguments)]
pub fn build_snapshot(
    mut agents: Vec<AgentPane>,
    tmux_statuses: &HashMap<String, Option<String>>,
    pane_window_ids: &HashMap<String, String>,
    active_windows: HashSet<(String, String)>,
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

    let snapshot_agents: Vec<SnapshotAgent> = agents
        .iter()
        .map(|a| {
            let window_id = pane_window_ids.get(&a.pane_id).cloned().unwrap_or_default();
            SnapshotAgent {
                pane_id: a.pane_id.clone(),
                session: a.session.clone(),
                window_name: a.window_name.clone(),
                window_id,
                path: a.path.clone(),
                pane_title: a.pane_title.clone(),
                status: a.status,
                status_ts: a.status_ts,
            }
        })
        .collect();

    SidebarSnapshot {
        version,
        layout_mode,
        active_windows,
        window_pane_counts,
        agents: snapshot_agents,
    }
}
