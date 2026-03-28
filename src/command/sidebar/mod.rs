//! Sidebar TUI for monitoring active workmux agents.
//!
//! Uses a daemon process that polls tmux and pushes state snapshots to
//! render-only sidebar clients via Unix socket. Each sidebar pane connects
//! to the daemon and receives updates, enabling instant window-switch response
//! without per-pane polling.
//!
//! # Module structure
//!
//! - `app` - application state and selection logic
//! - `client` - Unix socket client for receiving daemon snapshots
//! - `daemon` - background process that polls tmux and broadcasts snapshots
//! - `daemon_ctrl` - daemon lifecycle (spawn, kill, signal, health checks)
//! - `hooks` - tmux hook installation and removal
//! - `layout` - window layout save/restore for toggle cycles
//! - `layout_tree` - tmux layout tree parser and reflow for balanced pane widths
//! - `panes` - sidebar pane creation, destruction, and shutdown
//! - `runtime` - TUI event loop
//! - `snapshot` - snapshot data types and builder
//! - `ui` - ratatui rendering (compact and tile layouts)

mod app;
mod client;
mod daemon;
mod daemon_ctrl;
mod hooks;
mod layout;
mod layout_tree;
mod panes;
mod runtime;
mod snapshot;
mod ui;

use anyhow::{Result, anyhow};

use crate::cmd::Cmd;

use self::daemon_ctrl::{ensure_daemon_running, kill_daemon, signal_daemon};
use self::hooks::{install_hooks, remove_hooks};
use self::panes::{
    create_sidebar_in_window, create_sidebars_in_all_windows, find_sidebar_in_window,
    kill_all_sidebars_and_restore_layouts,
};

const SIDEBAR_ROLE_VALUE: &str = "sidebar";
const MIN_WIDTH: u16 = 25;
const MAX_WIDTH: u16 = 50;

/// Global tmux options set while the sidebar is active.
const SIDEBAR_GLOBAL_OPTIONS: &[&str] = &[
    "@workmux_sidebar_enabled",
    "@workmux_sidebar_width",
    "@workmux_sidebar_agents",
];

/// Unset all sidebar global tmux options.
fn clear_sidebar_globals() {
    for opt in SIDEBAR_GLOBAL_OPTIONS {
        let _ = Cmd::new("tmux").args(&["set-option", "-gu", opt]).run();
    }
}

/// Get the tmux client width in columns.
fn terminal_width() -> u16 {
    Cmd::new("tmux")
        .args(&["display-message", "-p", "#{client_width}"])
        .run_and_capture_stdout()
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Resolve sidebar width from config, falling back to 10% of terminal (clamped 25-50).
fn resolve_width(config: &crate::config::Config) -> u16 {
    resolve_width_for(config, terminal_width())
}

/// Resolve sidebar width for a given terminal/window width.
fn resolve_width_for(config: &crate::config::Config, tw: u16) -> u16 {
    if let Some(ref w) = config.sidebar.width {
        // Explicit config: respect it, only enforce a minimum of 10
        return w.resolve(tw).max(10);
    }

    // Default: 10% of terminal, clamped to [MIN_WIDTH, MAX_WIDTH]
    if tw == 0 {
        return MIN_WIDTH;
    }
    (tw * 10 / 100).clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Toggle the sidebar globally across all tmux windows.
pub fn toggle() -> Result<()> {
    let config = crate::config::Config::load(None)?;
    let width = resolve_width(&config);

    if std::env::var("TMUX").is_err() {
        return Err(anyhow!("Sidebar requires tmux"));
    }

    // Determine intent based on the current window's state
    let current_window = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{window_id}"])
        .run_and_capture_stdout()?
        .trim()
        .to_string();

    let current_has_sidebar = find_sidebar_in_window(&current_window).unwrap_or(false);

    if current_has_sidebar {
        // Current window has sidebar → toggle OFF globally
        kill_all_sidebars_and_restore_layouts();
        kill_daemon();
        remove_hooks();
        clear_sidebar_globals();
        return Ok(());
    }

    // Mark sidebar as used so the dashboard tip is dismissed
    let _ = std::thread::spawn(crate::tips::mark_sidebar_used);

    // Current window missing sidebar → enable/repair globally
    let width_str = width.to_string();
    Cmd::new("tmux")
        .args(&["set-option", "-g", "@workmux_sidebar_enabled", "1"])
        .run()?;
    Cmd::new("tmux")
        .args(&["set-option", "-g", "@workmux_sidebar_width", &width_str])
        .run()?;

    // Ensure daemon is running (spawns if needed)
    ensure_daemon_running()?;

    create_sidebars_in_all_windows(width)?;
    install_hooks()?;

    Ok(())
}

/// Sync sidebar into a window (called by tmux hooks for new windows/sessions).
pub fn sync(window_id: Option<&str>) -> Result<()> {
    if !is_sidebar_enabled() {
        return Ok(());
    }

    // Ensure daemon is running (may have auto-exited or crashed)
    let _ = ensure_daemon_running();

    // Use the provided window ID or fall back to current window
    let target = match window_id {
        Some(id) => id.to_string(),
        None => Cmd::new("tmux")
            .args(&["display-message", "-p", "#{window_id}"])
            .run_and_capture_stdout()?
            .trim()
            .to_string(),
    };

    if target.is_empty() {
        return Ok(());
    }

    // Check if this window already has a sidebar
    if find_sidebar_in_window(&target)? {
        return Ok(());
    }

    // Read width from tmux global (set by toggle) or fall back to config
    let width = Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@workmux_sidebar_width"])
        .run_and_capture_stdout()
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
        .unwrap_or_else(|| {
            let config = crate::config::Config::load(None).unwrap_or_default();
            resolve_width(&config)
        });
    create_sidebar_in_window(&target, width)?;

    Ok(())
}

/// Reflow sidebar layout after a window resize (called by tmux hook).
///
/// Finds the sidebar pane in the target window and runs the layout tree
/// reflow to keep the sidebar at the correct width and content panes balanced.
pub fn reflow(window_id: Option<&str>) -> Result<()> {
    if !is_sidebar_enabled() {
        return Ok(());
    }

    let target = match window_id {
        Some(id) => id.to_string(),
        None => Cmd::new("tmux")
            .args(&["display-message", "-p", "#{window_id}"])
            .run_and_capture_stdout()?
            .trim()
            .to_string(),
    };

    if target.is_empty() {
        return Ok(());
    }

    // Find the sidebar pane ID in this window
    let output = Cmd::new("tmux")
        .args(&[
            "list-panes",
            "-t",
            &target,
            "-F",
            "#{pane_id} #{@workmux_role}",
        ])
        .run_and_capture_stdout()?;

    let sidebar_pane_id = output.lines().find_map(|line| {
        let (id, role) = line.split_once(' ')?;
        (role.trim() == SIDEBAR_ROLE_VALUE).then(|| id.to_string())
    });

    let Some(sidebar_pane_id) = sidebar_pane_id else {
        return Ok(());
    };

    // Compute sidebar width based on the target window's width (not the client's,
    // since the window may belong to a different session with different dimensions)
    let config = crate::config::Config::load(None).unwrap_or_default();
    let window_w: u16 = Cmd::new("tmux")
        .args(&["display-message", "-t", &target, "-p", "#{window_width}"])
        .run_and_capture_stdout()
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let width = resolve_width_for(&config, window_w);

    layout_tree::reflow_after_sidebar_add(&target, &sidebar_pane_id, width);
    Ok(())
}

/// Run the sidebar daemon (called by the hidden `_sidebar-daemon` command).
pub fn run_daemon() -> Result<()> {
    daemon::run()
}

/// Run the sidebar TUI (called by the hidden `_sidebar-run` command).
pub fn run_sidebar() -> Result<()> {
    runtime::run_sidebar()
}

fn is_sidebar_enabled() -> bool {
    Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@workmux_sidebar_enabled"])
        .run_and_capture_stdout()
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

/// Navigation action for sidebar hotkeys.
pub enum NavAction {
    Next,
    Prev,
    Jump(usize),
}

/// Compute the target index for a navigation action given the current index and list length.
fn compute_nav_target(action: &NavAction, current_idx: Option<usize>, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some(match action {
        NavAction::Next => {
            let i = current_idx.unwrap_or(len - 1);
            if i >= len - 1 { 0 } else { i + 1 }
        }
        NavAction::Prev => {
            let i = current_idx.unwrap_or(0);
            if i == 0 { len - 1 } else { i - 1 }
        }
        NavAction::Jump(n) => {
            let idx = n - 1;
            if idx >= len {
                return None;
            }
            idx
        }
    })
}

/// Navigate to an agent by reading the daemon's ordered agent list from tmux.
pub fn navigate(action: NavAction) -> Result<()> {
    if std::env::var("TMUX").is_err() {
        return Err(anyhow!("Sidebar requires tmux"));
    }

    let agents_str = Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@workmux_sidebar_agents"])
        .run_and_capture_stdout()
        .unwrap_or_default();
    let agents_str = agents_str.trim();

    if agents_str.is_empty() {
        anyhow::bail!("no sidebar agents found (is the sidebar running?)");
    }

    // Parse space-separated pane IDs
    let panes: Vec<&str> = agents_str.split_whitespace().collect();

    if panes.is_empty() {
        anyhow::bail!("no sidebar agents found");
    }

    // Find current agent by active pane ID
    let current_pane_id = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{pane_id}"])
        .run_and_capture_stdout()
        .unwrap_or_default();
    let current_pane_id = current_pane_id.trim();

    let current_idx = panes.iter().position(|&pid| pid == current_pane_id);

    let len = panes.len();
    let target_idx = match &action {
        NavAction::Jump(n) => compute_nav_target(&action, current_idx, len)
            .ok_or_else(|| anyhow::anyhow!("agent {} out of range (1-{})", n, len))?,
        _ => compute_nav_target(&action, current_idx, len)
            .expect("len > 0 guarantees a result for Next/Prev"),
    };

    let target_pane = panes[target_idx];
    Cmd::new("tmux")
        .args(&["switch-client", "-t", target_pane])
        .run()?;

    signal_daemon();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_wraps_from_last_to_first() {
        assert_eq!(compute_nav_target(&NavAction::Next, Some(2), 3), Some(0));
    }

    #[test]
    fn next_advances_normally() {
        assert_eq!(compute_nav_target(&NavAction::Next, Some(0), 3), Some(1));
        assert_eq!(compute_nav_target(&NavAction::Next, Some(1), 3), Some(2));
    }

    #[test]
    fn next_without_current_wraps_from_last() {
        // No current window match: starts from last, wraps to first
        assert_eq!(compute_nav_target(&NavAction::Next, None, 3), Some(0));
    }

    #[test]
    fn prev_wraps_from_first_to_last() {
        assert_eq!(compute_nav_target(&NavAction::Prev, Some(0), 3), Some(2));
    }

    #[test]
    fn prev_goes_back_normally() {
        assert_eq!(compute_nav_target(&NavAction::Prev, Some(2), 3), Some(1));
        assert_eq!(compute_nav_target(&NavAction::Prev, Some(1), 3), Some(0));
    }

    #[test]
    fn prev_without_current_wraps_to_last() {
        // No current window match: starts from 0, wraps to last
        assert_eq!(compute_nav_target(&NavAction::Prev, None, 3), Some(2));
    }

    #[test]
    fn jump_converts_1_indexed_to_0_indexed() {
        assert_eq!(compute_nav_target(&NavAction::Jump(1), None, 3), Some(0));
        assert_eq!(compute_nav_target(&NavAction::Jump(2), None, 3), Some(1));
        assert_eq!(compute_nav_target(&NavAction::Jump(3), None, 3), Some(2));
    }

    #[test]
    fn jump_out_of_range_returns_none() {
        assert_eq!(compute_nav_target(&NavAction::Jump(4), None, 3), None);
        assert_eq!(compute_nav_target(&NavAction::Jump(10), None, 3), None);
    }

    #[test]
    fn empty_list_returns_none() {
        assert_eq!(compute_nav_target(&NavAction::Next, None, 0), None);
        assert_eq!(compute_nav_target(&NavAction::Prev, None, 0), None);
        assert_eq!(compute_nav_target(&NavAction::Jump(1), None, 0), None);
    }

    #[test]
    fn single_agent_next_stays() {
        assert_eq!(compute_nav_target(&NavAction::Next, Some(0), 1), Some(0));
    }

    #[test]
    fn single_agent_prev_stays() {
        assert_eq!(compute_nav_target(&NavAction::Prev, Some(0), 1), Some(0));
    }
}
