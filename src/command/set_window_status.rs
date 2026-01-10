use anyhow::Result;
use clap::ValueEnum;

use crate::cmd::Cmd;
use crate::config::Config;
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
    // Fail silently if not in tmux to avoid polluting non-tmux shells
    let Ok(pane) = std::env::var("TMUX_PANE") else {
        return Ok(());
    };

    let config = Config::load(None)?;

    // Ensure the status format is applied so the icon actually shows up
    // Skip for Clear since there's nothing to display
    if config.status_format.unwrap_or(true) && !matches!(cmd, SetWindowStatusCommand::Clear) {
        let _ = tmux::ensure_status_format(&pane);
    }

    match cmd {
        SetWindowStatusCommand::Working => set_status(&pane, config.status_icons.working()),
        SetWindowStatusCommand::Waiting => {
            set_status_with_auto_clear(&pane, config.status_icons.waiting())
        }
        SetWindowStatusCommand::Done => {
            set_status_with_auto_clear(&pane, config.status_icons.done())
        }
        SetWindowStatusCommand::Clear => clear_status(&pane),
    }
}

fn set_status(pane: &str, icon: &str) -> Result<()> {
    tmux::set_status_options(pane, icon, true);
    Ok(())
}

fn set_status_with_auto_clear(pane: &str, icon: &str) -> Result<()> {
    tmux::set_status_options(pane, icon, true);

    // Attach hook to clear window status on focus (only if status still matches the icon)
    // Uses tmux conditional: if @workmux_status equals the icon, clear window options
    // Note: Pane options are NOT cleared - they persist for status popup/dashboard tracking
    let hook_cmd = format!(
        "if-shell -F \"#{{==:#{{@workmux_status}},{}}}\" \
         \"set-option -uw @workmux_status ; \
           set-option -uw @workmux_status_ts\"",
        icon
    );

    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-w", "-t", pane, "pane-focus-in", &hook_cmd])
        .run();

    Ok(())
}

fn clear_status(pane: &str) -> Result<()> {
    // Clear Window Options
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-uw", "-t", pane, "@workmux_status"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-uw", "-t", pane, "@workmux_status_ts"])
        .run();

    // Clear Pane Options
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-up", "-t", pane, "@workmux_pane_status"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-up", "-t", pane, "@workmux_pane_status_ts"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-up", "-t", pane, "@workmux_pane_command"])
        .run();

    Ok(())
}
