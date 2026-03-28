//! Window layout save/restore for sidebar toggle cycles.

use crate::cmd::Cmd;

/// Save a window's layout to a tmux window option.
pub(super) fn save_window_layout(window_id: &str) {
    if let Ok(layout) = Cmd::new("tmux")
        .args(&["display-message", "-t", window_id, "-p", "#{window_layout}"])
        .run_and_capture_stdout()
    {
        let layout = layout.trim();
        if !layout.is_empty() {
            let _ = Cmd::new("tmux")
                .args(&[
                    "set-option",
                    "-w",
                    "-t",
                    window_id,
                    "@workmux_sidebar_layout",
                    layout,
                ])
                .run();
        }
    }
}

/// Restore a window's layout from the saved tmux window option.
pub(super) fn restore_window_layout(window_id: &str) {
    if let Ok(layout) = Cmd::new("tmux")
        .args(&[
            "show-option",
            "-wqv",
            "-t",
            window_id,
            "@workmux_sidebar_layout",
        ])
        .run_and_capture_stdout()
    {
        let layout = layout.trim();
        if !layout.is_empty() {
            let _ = Cmd::new("tmux")
                .args(&["select-layout", "-t", window_id, layout])
                .run();
            let _ = Cmd::new("tmux")
                .args(&[
                    "set-option",
                    "-wu",
                    "-t",
                    window_id,
                    "@workmux_sidebar_layout",
                ])
                .run();
        }
    }
}
