//! Tmux hook installation and removal for sidebar lifecycle events.

use anyhow::{Result, anyhow};

use crate::cmd::Cmd;

use super::{MAX_WIDTH, MIN_WIDTH};

/// Install tmux hooks so new windows automatically get a sidebar.
pub(super) fn install_hooks() -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_str().ok_or_else(|| anyhow!("exe path not UTF-8"))?;

    let sync_cmd = format!(
        "run-shell -b '{} _sidebar-sync --window #{{window_id}}'",
        exe_str
    );

    Cmd::new("tmux")
        .args(&["set-hook", "-g", "after-new-window[99]", &sync_cmd])
        .run()?;
    Cmd::new("tmux")
        .args(&["set-hook", "-g", "after-new-session[99]", &sync_cmd])
        .run()?;

    // Snap sidebar panes to responsive width when any window resizes.
    // window-resized fires on terminal resize AND when switching to an unattached
    // session (window-size=latest resizes windows to match the new client).
    // Scoped to the specific window that resized using ##{{window_id}}.
    // Double ## escapes tmux format expansion in hook values: ## → literal #
    let resize_script = format!(
        r#"ww=$(tmux display-message -t '##{{window_id}}' -p '##{{window_width}}'); w=$((ww * 10 / 100)); [ "$w" -lt {min} ] && w={min}; [ "$w" -gt {max} ] && w={max}; tmux list-panes -t '##{{window_id}}' -F '##{{pane_id}} ##{{@workmux_role}} ##{{pane_width}}' | while read id role cur; do [ "$role" = sidebar ] && [ "$cur" != "$w" ] && tmux resize-pane -t "$id" -x "$w"; done; true"#,
        min = MIN_WIDTH,
        max = MAX_WIDTH,
    );
    let resize_cmd = format!("run-shell -b \"{}\"", resize_script);
    Cmd::new("tmux")
        .args(&["set-hook", "-g", "window-resized[99]", &resize_cmd])
        .run()?;

    // Dirty signal hooks: send SIGUSR1 to daemon on window/session/pane changes
    let dirty_cmd = "run-shell -b 'kill -USR1 $(tmux show-option -gqv @workmux_sidebar_daemon_pid) 2>/dev/null'";
    Cmd::new("tmux")
        .args(&["set-hook", "-g", "after-select-window[98]", dirty_cmd])
        .run()?;
    Cmd::new("tmux")
        .args(&["set-hook", "-g", "client-session-changed[98]", dirty_cmd])
        .run()?;
    Cmd::new("tmux")
        .args(&["set-hook", "-g", "after-kill-pane[98]", dirty_cmd])
        .run()?;

    Ok(())
}

/// Remove tmux hooks.
pub(super) fn remove_hooks() {
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "after-new-window[99]"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "after-new-session[99]"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "window-resized[99]"])
        .run();
    // Dirty signal hooks
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "after-select-window[98]"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "client-session-changed[98]"])
        .run();
    let _ = Cmd::new("tmux")
        .args(&["set-hook", "-gu", "after-kill-pane[98]"])
        .run();
}
