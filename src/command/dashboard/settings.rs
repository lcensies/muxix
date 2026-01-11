//! Tmux-persisted dashboard settings.

use crate::cmd::Cmd;

const TMUX_HIDE_STALE_VAR: &str = "@workmux_hide_stale";
const TMUX_PREVIEW_SIZE_VAR: &str = "@workmux_preview_size";

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

/// Load preview size from tmux global variable.
/// Returns None if not set (so config default can be used).
pub fn load_preview_size_from_tmux() -> Option<u8> {
    Cmd::new("tmux")
        .args(&["show-option", "-gqv", TMUX_PREVIEW_SIZE_VAR])
        .run_and_capture_stdout()
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| s.trim().parse().ok())
}

/// Save preview size to tmux global variable
pub fn save_preview_size_to_tmux(size: u8) {
    let _ = Cmd::new("tmux")
        .args(&["set-option", "-g", TMUX_PREVIEW_SIZE_VAR, &size.to_string()])
        .run();
}
