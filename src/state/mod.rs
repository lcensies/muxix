//! Filesystem-based state storage for muxix agents.
//!
//! This module provides persistent state storage that works across all
//! terminal multiplexer backends (tmux, WezTerm, Zellij).

pub mod codex_status;
pub mod run;
pub mod store;
mod types;

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use tracing::warn;

use crate::agent::identity::classify_agent_kind;
use crate::multiplexer::{AgentStatus, Multiplexer};

pub use store::StateStore;
pub use types::{AgentState, Completion, CompletionKind, LastDoneCycleState, PaneKey, RuntimeState};

/// Persist an agent state update to the StateStore.
///
/// Merges with existing state so partial updates don't wipe other fields:
/// - If `status` is Some, updates the agent's status. If None, preserves existing.
/// - If `title_override` is Some, uses it. If None, preserves existing stored title,
///   falling back to the live pane title.
///
/// Logs warnings on failure without propagating errors (best-effort persistence).
pub fn persist_agent_update(
    mux: &dyn Multiplexer,
    pane_id: &str,
    status: Option<AgentStatus>,
    title_override: Option<String>,
) {
    let pane_key = PaneKey {
        backend: mux.name().to_string(),
        instance: mux.instance_id(),
        pane_id: pane_id.to_string(),
    };

    let live_info = match mux.get_live_pane_info(pane_id) {
        Ok(Some(info)) => info,
        Ok(None) => {
            warn!(%pane_id, "pane not found, skipping state persist");
            return;
        }
        Err(e) => {
            warn!(error = %e, "failed to get live pane info, skipping state persist");
            return;
        }
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Load existing state to merge with
    let existing = StateStore::new()
        .ok()
        .and_then(|store| store.get_agent(&pane_key).ok().flatten());

    // Resolve status: explicit update wins, otherwise preserve existing
    let final_status = status.or(existing.as_ref().and_then(|e| e.status));

    // Preserve existing status_ts if status hasn't changed (avoids resetting timer)
    let status_ts = if final_status == existing.as_ref().and_then(|e| e.status) {
        existing.as_ref().and_then(|e| e.status_ts).unwrap_or(now)
    } else {
        now
    };

    // Capture fields from `existing` before it is consumed by `and_then` below.
    let existing_agent_kind = existing.as_ref().and_then(|e| e.agent_kind.clone());
    // Preserve agent_id across status/title updates; generate once on first registration.
    let agent_id = existing
        .as_ref()
        .map(|e| e.agent_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    // Preserve sandbox / checkpoint metadata: it is written only by the
    // dedicated sandbox commands, never by this status-update path, so it
    // must be carried over from existing state or it would be wiped. The
    // sandbox id is first established at launch via the pane→sandbox sidecar,
    // so adopt that when existing state has none yet.
    let sandbox_id = existing
        .as_ref()
        .and_then(|e| e.sandbox_id.clone())
        .or_else(|| {
            StateStore::new()
                .ok()
                .and_then(|store| store.pane_sandbox(&pane_key))
        });
    let checkpoint_path = existing.as_ref().and_then(|e| e.checkpoint_path.clone());
    let checkpoint_ts = existing.as_ref().and_then(|e| e.checkpoint_ts);
    // Preserve pipeline node fields written by the runner; never cleared by the status-update path.
    let pipeline_node_id = existing.as_ref().and_then(|e| e.pipeline_node_id.clone());
    let pipeline_node_title = existing.as_ref().and_then(|e| e.pipeline_node_title.clone());
    // Same for the agent-authored activity label: only `signal activity` owns it.
    let activity = existing.as_ref().and_then(|e| e.activity.clone());
    // Preserve completion: this path is a hook-driven status/title update, not
    // a launch or send, so it must never clear an agent's completion claim.
    let existing_completion = existing.as_ref().and_then(|e| e.completion.clone());

    // Snapshot the live title for classification before the resolved
    // `pane_title` consumes `live_info.title`.
    let live_title_for_classify = live_info.title.clone();

    // Resolve title: explicit override wins, then existing stored title, then live
    let pane_title = title_override
        .or(existing.and_then(|e| e.pane_title))
        .or(live_info.title);

    // Get server boot ID for crash detection (best-effort)
    let boot_id = mux.server_boot_id().unwrap_or(None);

    // Classify the agent kind once and lock it in. The classifier sees the
    // *live* title (not the merged `pane_title` above, which prefers the
    // stored value): a stale stored title would otherwise re-confirm the
    // previous identity even after the foreground command has changed.
    // Pane reuse (Claude exits, another agent launches in the same pane) is
    // handled by reconcile in `state::store`, which deletes the stored
    // entry on `command` change before this path runs again.
    let agent_kind = merge_agent_kind(
        classify_agent_kind(
            live_info.current_command.as_deref(),
            live_title_for_classify.as_deref(),
        ),
        existing_agent_kind,
    );

    let state = AgentState {
        runtime: None,
        agent_id,
        pane_key,
        workdir: live_info.working_dir,
        status: final_status,
        status_ts: Some(status_ts),
        pane_title,
        pane_pid: live_info.pid.unwrap_or(0),
        command: live_info.current_command.unwrap_or_default(),
        updated_ts: now,
        window_name: live_info.window,
        session_name: live_info.session,
        boot_id,
        agent_kind,
        sandbox_id,
        checkpoint_path,
        checkpoint_ts,
        pipeline_node_id,
        pipeline_node_title,
        activity,
        completion: existing_completion,
    };

    if let Ok(store) = StateStore::new()
        && let Err(e) = store.upsert_agent(&state)
    {
        warn!(error = %e, "failed to persist agent state");
    }
}

/// Persist an agent-authored completion claim (or clear one) for a pane.
///
/// Mirrors `persist_agent_update`'s merge shape but touches only `completion`
/// and `updated_ts`: load the existing record for the pane, set `completion`,
/// bump `updated_ts`, save. Unlike `persist_agent_update`, this does not
/// consult live pane info — the pane may have gone away between the agent
/// emitting the signal and this call landing, and completion is meaningful
/// even for a now-dead pane. If no record exists yet for the pane, there is
/// nothing to merge into; warn and return.
pub fn persist_agent_completion(
    mux: &dyn Multiplexer,
    pane_id: &str,
    completion: Option<Completion>,
) {
    let pane_key = PaneKey {
        backend: mux.name().to_string(),
        instance: mux.instance_id(),
        pane_id: pane_id.to_string(),
    };

    let Some(store) = StateStore::new().ok() else {
        warn!(%pane_id, "failed to open state store, skipping completion persist");
        return;
    };

    let Some(mut state) = store.get_agent(&pane_key).ok().flatten() else {
        warn!(%pane_id, "no live agent info for pane, skipping completion persist");
        return;
    };

    state.completion = completion;
    state.updated_ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if let Err(e) = store.upsert_agent(&state) {
        warn!(error = %e, "failed to persist agent completion");
    }
}

/// Build a `pane_id -> Completion` lookup for the current backend/instance.
///
/// `wait`/`status` match agents via `AgentPane` (derived from `AgentState` but
/// omitting fields nothing else needs); widening `AgentPane` with `completion`
/// would touch every test fixture that constructs it as a struct literal
/// (sidebar, dashboard, ...). Looking it up separately here is cheaper and
/// keeps `AgentPane` unchanged.
pub fn completion_by_pane(mux: &dyn Multiplexer) -> HashMap<String, Completion> {
    let backend = mux.name();
    let instance = mux.instance_id();
    StateStore::new()
        .and_then(|store| store.list_all_agents())
        .map(|agents| {
            agents
                .into_iter()
                .filter(|a| a.pane_key.backend == backend && a.pane_key.instance == instance)
                .filter_map(|a| a.completion.map(|c| (a.pane_key.pane_id.clone(), c)))
                .collect()
        })
        .unwrap_or_default()
}

/// Merge a freshly classified agent kind with the previously cached one.
///
/// Locks in the first definitive answer: once `existing` is `Some(_)`, that
/// value is preserved. This guards against title drift (a non-agent process
/// printing a substring like "Vibe" or "◇" into the pane title and stealing
/// the cached identity). Pane reuse is handled separately by reconcile,
/// which removes the stored entry when `pane_current_command` changes.
fn merge_agent_kind(new: Option<String>, existing: Option<String>) -> Option<String> {
    existing.or(new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_existing_when_new_is_none() {
        let merged = merge_agent_kind(None, Some("claude".into()));
        assert_eq!(merged, Some("claude".into()));
    }

    #[test]
    fn merge_preserves_existing_against_drift() {
        // Existing was correctly classified; a later tick whose title drifted
        // into another agent's fingerprint must not overwrite it.
        let merged = merge_agent_kind(Some("vibe".into()), Some("claude".into()));
        assert_eq!(merged, Some("claude".into()));
    }

    #[test]
    fn merge_returns_none_when_both_none() {
        assert_eq!(merge_agent_kind(None, None), None);
    }

    #[test]
    fn merge_classifies_when_existing_is_none() {
        let merged = merge_agent_kind(Some("claude".into()), None);
        assert_eq!(merged, Some("claude".into()));
    }
}
