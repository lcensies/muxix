use anyhow::Result;
use clap::ValueEnum;

use crate::config::Config;
use crate::multiplexer::{create_backend, detect_backend};
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
    let Some(pane) = mux.current_pane_id() else {
        return Ok(());
    };

    // Ensure the status format is applied so the icon actually shows up
    // Skip for Clear since there's nothing to display
    if config.status_format.unwrap_or(true) && !matches!(cmd, SetWindowStatusCommand::Clear) {
        let _ = mux.ensure_status_format(&pane);
    }

    match cmd {
        SetWindowStatusCommand::Working => {
            tmux::pop_done_pane(&pane); // Remove from done stack
            mux.set_status(&pane, config.status_icons.working(), true)?;
        }
        SetWindowStatusCommand::Waiting => {
            tmux::pop_done_pane(&pane); // Remove from done stack
            mux.set_status(&pane, config.status_icons.waiting(), true)?;
        }
        SetWindowStatusCommand::Done => {
            tmux::push_done_pane(&pane); // Add to done stack (most recent)
            mux.set_status(&pane, config.status_icons.done(), true)?;
        }
        SetWindowStatusCommand::Clear => {
            tmux::pop_done_pane(&pane); // Remove from done stack
            mux.clear_status(&pane)?;
        }
    }

    Ok(())
}
