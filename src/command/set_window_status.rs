use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use clap::ValueEnum;
use tracing::warn;

use crate::config::Config;
use crate::multiplexer::{create_backend, detect_backend};
use crate::state::{AgentState, PaneKey, StateStore};
use crate::tmux;

#[derive(ValueEnum, Debug, Clone)]
pub enum SetWindowStatusCommand {
    /// Set status to "working" (agent is processing)
    Working,
    /// Set status to "waiting" (agent needs user input) - auto-clears on window focus
    Waiting,
    /// Set status to "done" (agent finished) - auto-clears on window focus
    Done,
    /// Clear the status
    Clear,
}

pub fn run(cmd: SetWindowStatusCommand) -> Result<()> {
    let config = Config::load(None)?;
    let mux = create_backend(detect_backend(&config));

    // Fail silently if not in a multiplexer session
    let Some(pane_id) = mux.current_pane_id() else {
        return Ok(());
    };

    // Build pane key for state storage
    let pane_key = PaneKey {
        backend: mux.name().to_string(),
        instance: mux.instance_id(),
        pane_id: pane_id.clone(),
    };

    match cmd {
        SetWindowStatusCommand::Clear => {
            // Remove from done stack (tmux-specific)
            tmux::pop_done_pane(&pane_id);

            // Delete state file
            if let Ok(store) = StateStore::new()
                && let Err(e) = store.delete_agent(&pane_key) {
                    warn!(error = %e, "failed to delete agent state");
                }

            // Clear backend UI
            mux.clear_status(&pane_id)?;
        }
        SetWindowStatusCommand::Working
        | SetWindowStatusCommand::Waiting
        | SetWindowStatusCommand::Done => {
            let icon = match cmd {
                SetWindowStatusCommand::Working => config.status_icons.working(),
                SetWindowStatusCommand::Waiting => config.status_icons.waiting(),
                SetWindowStatusCommand::Done => config.status_icons.done(),
                SetWindowStatusCommand::Clear => unreachable!(),
            };

            // Manage done stack for fast last-done cycling (tmux-specific)
            match cmd {
                SetWindowStatusCommand::Done => tmux::push_done_pane(&pane_id),
                SetWindowStatusCommand::Working | SetWindowStatusCommand::Waiting => {
                    tmux::pop_done_pane(&pane_id)
                }
                SetWindowStatusCommand::Clear => unreachable!(),
            }

            // Ensure the status format is applied so the icon actually shows up
            if config.status_format.unwrap_or(true) {
                let _ = mux.ensure_status_format(&pane_id);
            }

            // Get live pane info for PID and command
            if let Ok(Some(live_info)) = mux.get_live_pane_info(&pane_id) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                let state = AgentState {
                    pane_key,
                    workdir: live_info.working_dir,
                    status: Some(icon.to_string()),
                    status_ts: Some(now),
                    pane_title: live_info.title,
                    pane_pid: live_info.pid,
                    command: live_info.current_command,
                    updated_ts: now,
                };

                // Write to state store (don't fail the command if this fails)
                if let Ok(store) = StateStore::new()
                    && let Err(e) = store.upsert_agent(&state) {
                        warn!(error = %e, "failed to persist agent state");
                    }
            }

            // Update backend UI (status bar icon)
            // exit_detection=false since exit detection now uses StateStore
            mux.set_status(&pane_id, icon, false)?;
        }
    }

    Ok(())
}
