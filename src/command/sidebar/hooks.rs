//! Tmux hook installation and removal for sidebar lifecycle events.

use anyhow::{Result, anyhow};

use crate::cmd::Cmd;

/// All hook names installed by the sidebar.
const HOOK_NAMES: &[&str] = &[
    "after-new-window[99]",
    "after-new-session[99]",
    "after-split-window[99]",
    "window-linked[99]",
    "window-resized[99]",
    "after-select-window[98]",
    "client-session-changed[98]",
    "after-kill-pane[98]",
    "pane-focus-in[98]",
    "after-new-window[97]",
    "after-new-session[97]",
    "after-split-window[97]",
    "pane-exited[98]",
    "pane-died[98]",
    "window-unlinked[98]",
    "session-closed[98]",
];

/// Install tmux hooks so new windows automatically get a sidebar.
pub(super) fn install_hooks() -> Result<()> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_str().ok_or_else(|| anyhow!("exe path not UTF-8"))?;

    let sync_cmd = format!(
        "run-shell -b '{} _sidebar-sync --window #{{window_id}}'",
        exe_str
    );

    // Reflow sidebar layouts in all windows when any window resizes.
    // This ensures inactive windows get corrected without waiting for the
    // user to visit them. window-resized fires on terminal resize AND when
    // switching to an unattached session (window-size=latest resizes windows
    // to match the new client).
    let reflow_cmd = format!("run-shell -b '{} _sidebar-reflow-all'", exe_str);

    // Re-assert the sidebar layout right after an external tiling manager
    // (e.g. tmux-tilish) rebalances the window. tilish installs index-0
    // `after-split-window` / `window-linked` hooks that force the configured
    // layout (`select-layout ; select-layout -E`), which stomps the sidebar to
    // ~50%. Our hooks run at index 99, i.e. AFTER tilish's, so the sidebar
    // width wins on every split/link without racing a fixed timer.
    let reflow_window_cmd = format!(
        "run-shell -b '{} _sidebar-reflow --window #{{window_id}}'",
        exe_str
    );

    // Dirty signal: send SIGUSR1 to daemon on window/session/pane changes
    let dirty_cmd = "run-shell -b 'kill -USR1 $(tmux show-option -gqv @workmux_sidebar_daemon_pid) 2>/dev/null || true'";

    let hooks: &[(&str, &str)] = &[
        ("after-new-window[99]", &sync_cmd),
        ("after-new-session[99]", &sync_cmd),
        ("after-split-window[99]", &reflow_window_cmd),
        ("window-linked[99]", &reflow_cmd),
        ("window-resized[99]", &reflow_cmd),
        ("after-select-window[98]", dirty_cmd),
        ("client-session-changed[98]", dirty_cmd),
        ("after-kill-pane[98]", dirty_cmd),
        ("pane-focus-in[98]", dirty_cmd),
        // Pane/window lifecycle pokes: the daemon is event-driven and only
        // sweeps every 30s on its own, so anything that adds or removes panes
        // must signal it for the sidebar to update promptly.
        ("after-new-window[97]", dirty_cmd),
        ("after-new-session[97]", dirty_cmd),
        ("after-split-window[97]", dirty_cmd),
        ("pane-exited[98]", dirty_cmd),
        ("pane-died[98]", dirty_cmd),
        // kill-window has no after- hook (the window is gone before it could
        // fire; tmux 3.6 rejects the name outright). window-unlinked is the
        // event a killed window actually emits -- it leaves its session.
        ("window-unlinked[98]", dirty_cmd),
        ("session-closed[98]", dirty_cmd),
    ];

    for (hook, cmd) in hooks {
        Cmd::new("tmux")
            .args(&["set-hook", "-g", hook, cmd])
            .run()?;
    }

    Ok(())
}

/// Remove tmux hooks.
pub(super) fn remove_hooks() {
    for hook in HOOK_NAMES {
        let _ = Cmd::new("tmux").args(&["set-hook", "-gu", hook]).run();
    }
}
