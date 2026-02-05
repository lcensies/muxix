use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use clap::ValueEnum;
use tracing::warn;

use crate::config::Config;
use crate::multiplexer::{AgentStatus, create_backend, detect_backend};
use crate::state::{AgentState, PaneKey, StateStore};

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
    // Inside a sandbox guest, route through RPC to the host supervisor
    if crate::sandbox::guest::is_sandbox_guest() {
        return run_via_rpc(cmd);
    }

    let config = Config::load(None)?;
    let mux = create_backend(detect_backend());

    // Fail silently if not in a multiplexer session
    let Some(pane_id) = mux.current_pane_id() else {
        return Ok(());
    };

    match cmd {
        SetWindowStatusCommand::Clear => {
            // Clear icon only - state file cleanup is handled by reconciliation
            mux.clear_status(&pane_id)?;
        }
        SetWindowStatusCommand::Working
        | SetWindowStatusCommand::Waiting
        | SetWindowStatusCommand::Done => {
            let (status, icon, auto_clear) = match cmd {
                SetWindowStatusCommand::Working => {
                    (AgentStatus::Working, config.status_icons.working(), false)
                }
                SetWindowStatusCommand::Waiting => {
                    (AgentStatus::Waiting, config.status_icons.waiting(), true)
                }
                SetWindowStatusCommand::Done => {
                    (AgentStatus::Done, config.status_icons.done(), true)
                }
                SetWindowStatusCommand::Clear => unreachable!(),
            };

            let pane_key = PaneKey {
                backend: mux.name().to_string(),
                instance: mux.instance_id(),
                pane_id: pane_id.clone(),
            };

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

                // Preserve existing status_ts if status hasn't changed
                // This prevents timer reset when agent repeatedly reports same status
                let status_ts = StateStore::new()
                    .ok()
                    .and_then(|store| store.get_agent(&pane_key).ok().flatten())
                    .filter(|existing| existing.status == Some(status))
                    .and_then(|existing| existing.status_ts)
                    .unwrap_or(now);

                let state = AgentState {
                    pane_key,
                    workdir: live_info.working_dir,
                    status: Some(status),
                    status_ts: Some(status_ts),
                    pane_title: live_info.title,
                    pane_pid: live_info.pid,
                    command: live_info.current_command,
                    updated_ts: now,
                };

                // Write to state store (don't fail the command if this fails)
                if let Ok(store) = StateStore::new()
                    && let Err(e) = store.upsert_agent(&state)
                {
                    warn!(error = %e, "failed to persist agent state");
                }
            }

            // Update backend UI (status bar icon)
            mux.set_status(&pane_id, icon, auto_clear)?;
        }
    }

    Ok(())
}

/// Send a status update via RPC when running inside a sandbox guest.
fn run_via_rpc(cmd: SetWindowStatusCommand) -> Result<()> {
    use crate::sandbox::rpc::{RpcClient, RpcRequest, RpcResponse};

    let status = match cmd {
        SetWindowStatusCommand::Working => "working",
        SetWindowStatusCommand::Waiting => "waiting",
        SetWindowStatusCommand::Done => "done",
        SetWindowStatusCommand::Clear => "clear",
    };

    let mut client = RpcClient::from_env()?;
    let response = client.call(&RpcRequest::SetStatus {
        status: status.to_string(),
    })?;

    match response {
        RpcResponse::Ok => Ok(()),
        RpcResponse::Error { message } => {
            warn!(error = %message, "RPC SetStatus failed");
            Ok(()) // Fail silently like the host path does
        }
    }
}
