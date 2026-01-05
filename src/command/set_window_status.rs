use anyhow::Result;
use clap::ValueEnum;
use serde::Deserialize;
use std::io::{self, Read};

use crate::cmd::Cmd;
use crate::config::Config;
use crate::tmux;

#[derive(Deserialize)]
struct HookInput {
    notification_type: Option<String>,
}

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

    // Parse hook input from stdin (Claude Code passes JSON via stdin)
    let hook_input = read_hook_input();

    // Skip "waiting" status for idle_prompt notifications.
    // Claude sends idle_prompt if session is idle for some time. This is bad because it changes
    // the green checkmark to the speech bubble. Checkmark is much better at communicating "this
    // session is done for now", than the speech bubble. Speech bubble should stil come if user is
    // prompted for access or something
    if matches!(cmd, SetWindowStatusCommand::Waiting)
        && let Some(ref input) = hook_input
        && input.notification_type.as_deref() == Some("idle_prompt")
    {
        return Ok(());
    }

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

fn read_hook_input() -> Option<HookInput> {
    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer).ok()?;
    serde_json::from_str(&buffer).ok()
}

fn set_status(pane: &str, icon: &str) -> Result<()> {
    if let Err(e) = Cmd::new("tmux")
        .args(&["set-option", "-w", "-t", pane, "@workmux_status", icon])
        .run()
    {
        eprintln!("workmux: failed to set window status: {}", e);
    }
    Ok(())
}

fn set_status_with_auto_clear(pane: &str, icon: &str) -> Result<()> {
    // Set the status icon
    if let Err(e) = Cmd::new("tmux")
        .args(&["set-option", "-w", "-t", pane, "@workmux_status", icon])
        .run()
    {
        eprintln!("workmux: failed to set window status: {}", e);
        return Ok(());
    }

    // Attach hook to clear on focus (only if status still matches the icon)
    // Uses tmux conditional: if @workmux_status equals the icon, unset it
    let hook_cmd = format!(
        "if-shell -F \"#{{==:#{{@workmux_status}},{}}}\" \"set-option -uw @workmux_status\"",
        icon
    );

    if let Err(e) = Cmd::new("tmux")
        .args(&["set-hook", "-w", "-t", pane, "pane-focus-in", &hook_cmd])
        .run()
    {
        eprintln!("workmux: failed to set auto-clear hook: {}", e);
    }

    Ok(())
}

fn clear_status(pane: &str) -> Result<()> {
    if let Err(e) = Cmd::new("tmux")
        .args(&["set-option", "-uw", "-t", pane, "@workmux_status"])
        .run()
    {
        eprintln!("workmux: failed to clear window status: {}", e);
    }
    Ok(())
}
