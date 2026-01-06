use anyhow::Result;
use clap::ValueEnum;
use serde::Deserialize;
use std::io::{self, Read};
use std::time::{SystemTime, UNIX_EPOCH};

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
        SetWindowStatusCommand::Waiting => set_status(&pane, config.status_icons.waiting()),
        SetWindowStatusCommand::Done => set_status(&pane, config.status_icons.done()),
        SetWindowStatusCommand::Clear => clear_status(&pane),
    }
}

fn read_hook_input() -> Option<HookInput> {
    let mut buffer = String::new();
    io::stdin().read_to_string(&mut buffer).ok()?;
    serde_json::from_str(&buffer).ok()
}

fn set_status(pane: &str, icon: &str) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let now_str = now.to_string();

    // 1. Set Window Option (for tmux status bar display)
    // "Last write wins" behavior for the window icon
    if let Err(e) = Cmd::new("tmux")
        .args(&["set-option", "-w", "-t", pane, "@workmux_status", icon])
        .run()
    {
        eprintln!("workmux: failed to set window status: {}", e);
    }
    let _ = Cmd::new("tmux")
        .args(&[
            "set-option",
            "-w",
            "-t",
            pane,
            "@workmux_status_ts",
            &now_str,
        ])
        .run();

    // 2. Set Pane Option (for dashboard tracking)
    // Use a DISTINCT key to avoid inheritance issues in list-panes
    if let Err(e) = Cmd::new("tmux")
        .args(&["set-option", "-p", "-t", pane, "@workmux_pane_status", icon])
        .run()
    {
        eprintln!("workmux: failed to set pane status: {}", e);
    }
    let _ = Cmd::new("tmux")
        .args(&[
            "set-option",
            "-p",
            "-t",
            pane,
            "@workmux_pane_status_ts",
            &now_str,
        ])
        .run();

    // 3. Store the current foreground command for agent exit detection
    // When the command changes (e.g., from "node" to "zsh"), we know the agent exited
    let current_cmd = tmux::get_pane_current_command(pane).unwrap_or_default();
    if !current_cmd.is_empty() {
        let _ = Cmd::new("tmux")
            .args(&[
                "set-option",
                "-p",
                "-t",
                pane,
                "@workmux_pane_command",
                &current_cmd,
            ])
            .run();
    }

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
