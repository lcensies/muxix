//! Tmux-persisted dashboard settings.

use crate::cmd::Cmd;

const TMUX_HIDE_STALE_VAR: &str = "@workmux_hide_stale";

/// Load hide_stale filter state from tmux global variable
pub fn load_hide_stale_from_tmux() -> bool {
    Cmd::new("tmux")
        .args(&["show-option", "-gqv", TMUX_HIDE_STALE_VAR])
        .run_and_capture_stdout()
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| s.trim() == "true")
        .unwrap_or(false)
}

/// Save hide_stale filter state to tmux global variable
pub fn save_hide_stale_to_tmux(hide_stale: bool) {
    let _ = Cmd::new("tmux")
        .args(&[
            "set-option",
            "-g",
            TMUX_HIDE_STALE_VAR,
            if hide_stale { "true" } else { "false" },
        ])
        .run();
}
