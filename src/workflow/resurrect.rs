use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use tracing::info;

use crate::config::{self, MuxMode};
use crate::git;
use crate::multiplexer::Multiplexer;
use crate::state::{PaneKey, StateStore};
use crate::util::canon_or_self;

/// Is this record a restore candidate?
///
/// A tombstoned reference never is: the user deleted that agent, and a restore
/// would relaunch it with `--fork-session` on the very conversation they threw
/// away. An `Exited` one is not either by default — with `persist_agents` every
/// agent that ever finished leaves one for ever, and nothing expires it.
fn restorable(
    agent: &crate::state::AgentState,
    tombstoned: &std::collections::BTreeSet<String>,
    include_exited: bool,
) -> bool {
    if tombstoned.contains(&agent.pane_key.pane_id) {
        return false;
    }
    include_exited || agent.status != Some(crate::multiplexer::AgentStatus::Exited)
}

#[derive(Debug)]
pub enum ResurrectAction {
    Restore,
    SkipAlreadyOpen,
    SkipMain,
}

#[derive(Debug)]
pub struct ResurrectCandidate {
    pub handle: String,
    pub action: ResurrectAction,
    pub stale_pane_keys: Vec<PaneKey>,
    pub mode: MuxMode,
    /// Canonical worktree root. Needed to probe the agent's session store and
    /// to locate the stored task prompt when no session can be resumed.
    pub worktree_path: PathBuf,
}

pub struct ResurrectPlan {
    pub candidates: Vec<ResurrectCandidate>,
    pub unmatched_states: usize,
}

/// Build a plan of what to restore based on stale agent state files.
///
/// Loads raw (non-reconciled) agent states and cross-references them against
/// existing git worktrees and live multiplexer state to determine which
/// worktrees need restoration.
pub fn plan(
    store: &StateStore,
    mux: &dyn Multiplexer,
    include_exited: bool,
) -> Result<ResurrectPlan> {
    let all_agents = store.list_all_agents()?;
    let backend = mux.name();
    let instance = mux.instance_id();

    info!(
        total_state_files = all_agents.len(),
        backend, instance, "resurrect:plan loading agent state files"
    );

    // With `persist_agents`, every agent that ever finished leaves a permanent
    // `Exited` record, and nothing expires it — so without this filter every
    // one of them is a restore candidate for ever. A tombstone always wins:
    // the user deleted that agent.
    let tombstoned = store.tombstones();

    // Filter to current backend/instance
    let relevant: Vec<_> = all_agents
        .into_iter()
        .filter(|a| a.pane_key.backend == backend && a.pane_key.instance == instance)
        .filter(|a| restorable(a, &tombstoned, include_exited))
        .collect();

    info!(
        relevant_count = relevant.len(),
        "resurrect:plan filtered to current backend/instance"
    );

    // Get worktrees for current repo
    let worktrees = git::list_worktrees()?;
    let main_root = git::get_main_worktree_root()?;
    let canon_main = canon_or_self(&main_root);

    // Build canonical worktree map: (canon_path, handle)
    let wt_map: Vec<(PathBuf, String)> = worktrees
        .iter()
        .map(|(path, _branch)| {
            let handle = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            (canon_or_self(path), handle)
        })
        .collect();

    // Get live mux state for skip detection
    let mux_windows = mux.get_all_window_names()?;
    let mux_sessions = mux.get_all_session_names()?;
    let config = config::Config::load(None)?;
    let prefix = config.window_prefix();

    // Use config default mode as fallback for worktrees with no stored mode,
    // matching the resolution logic in workflow::open
    let default_mode = config.mode();

    // Group agent states by matched worktree handle
    let mut by_handle: HashMap<String, (MuxMode, Vec<PaneKey>)> = HashMap::new();
    let mut unmatched_states = 0usize;

    for agent in relevant {
        let canon_agent = canon_or_self(&agent.workdir);

        // Find matching worktree using descendant path matching
        // (agent workdir may be a subdirectory of the worktree root)
        let matched = wt_map
            .iter()
            .find(|(canon_wt, _)| canon_agent == *canon_wt || canon_agent.starts_with(canon_wt));

        match matched {
            Some((_canon_wt, handle)) => {
                info!(
                    pane_id = %agent.pane_key.pane_id,
                    workdir = %agent.workdir.display(),
                    handle,
                    boot_id = ?agent.boot_id,
                    status = ?agent.status,
                    "resurrect:plan matched agent to worktree"
                );
                let mode = git::get_worktree_mode_opt(handle).unwrap_or(default_mode);
                by_handle
                    .entry(handle.clone())
                    .or_insert_with(|| (mode, Vec::new()))
                    .1
                    .push(agent.pane_key);
            }
            None => {
                info!(
                    pane_id = %agent.pane_key.pane_id,
                    workdir = %agent.workdir.display(),
                    "resurrect:plan no matching worktree (other project or removed)"
                );
                unmatched_states += 1;
            }
        }
    }

    // Determine action per handle
    let mut candidates = Vec::new();
    for (handle, (mode, pane_keys)) in by_handle {
        let canon_wt = wt_map
            .iter()
            .find(|(_, h)| *h == handle)
            .map(|(p, _)| p.clone())
            .unwrap_or_default();

        let action = if canon_wt == canon_main {
            ResurrectAction::SkipMain
        } else {
            let prefixed = crate::multiplexer::util::prefixed(prefix, &handle);
            let is_open = if mode == MuxMode::Session {
                mux_sessions.contains(&prefixed)
            } else {
                mux_windows.contains(&prefixed)
            };
            if is_open {
                ResurrectAction::SkipAlreadyOpen
            } else {
                ResurrectAction::Restore
            }
        };

        info!(
            handle,
            action = ?action,
            mode = ?mode,
            pane_count = pane_keys.len(),
            "resurrect:plan determined action for handle"
        );

        candidates.push(ResurrectCandidate {
            handle,
            action,
            stale_pane_keys: pane_keys,
            mode,
            worktree_path: canon_wt,
        });
    }

    // Sort by handle for deterministic output
    candidates.sort_by(|a, b| a.handle.cmp(&b.handle));

    Ok(ResurrectPlan {
        candidates,
        unmatched_states,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiplexer::AgentStatus;
    use std::collections::BTreeSet;

    fn record(pane_id: &str, status: Option<AgentStatus>) -> crate::state::AgentState {
        let mut state: crate::state::AgentState = serde_json::from_value(serde_json::json!({
            "agent_id": "a",
            "pane_key": {"backend": "tmux", "instance": "/tmp/s", "pane_id": pane_id},
            "workdir": "/repo/wt",
            "pane_pid": 1,
            "command": "node",
            "updated_ts": 0,
        }))
        .unwrap();
        state.status = status;
        state
    }

    #[test]
    fn restore_skips_exited_records_unless_asked() {
        let none = BTreeSet::new();
        assert!(restorable(
            &record("%1", Some(AgentStatus::Working)),
            &none,
            false
        ));
        assert!(restorable(&record("%1", None), &none, false));
        assert!(!restorable(
            &record("%1", Some(AgentStatus::Exited)),
            &none,
            false
        ));
        assert!(restorable(
            &record("%1", Some(AgentStatus::Exited)),
            &none,
            true
        ));
    }

    #[test]
    fn a_tombstone_beats_include_exited() {
        let deleted = BTreeSet::from(["%1".to_string()]);
        for status in [None, Some(AgentStatus::Working), Some(AgentStatus::Exited)] {
            assert!(
                !restorable(&record("%1", status), &deleted, true),
                "{status:?}"
            );
        }
        assert!(restorable(&record("%2", None), &deleted, false));
    }
}
