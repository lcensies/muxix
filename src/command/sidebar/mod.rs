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
//! - `layout_tree` - tmux layout tree parser, reflow, and sidebar removal
//! - `panes` - sidebar pane creation, destruction, and shutdown
//! - `runtime` - TUI event loop
//! - `snapshot` - snapshot data types and builder
//! - `ui` - ratatui rendering (compact and tile layouts)

mod app;
mod client;
mod daemon;
mod daemon_ctrl;
mod hooks;
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
    create_sidebar_in_window, create_sidebars_in_all_windows, create_sidebars_in_session,
    find_sidebar_in_window, kill_all_sidebars_and_restore_layouts, kill_sidebars_in_session,
};

const SIDEBAR_ROLE_VALUE: &str = "sidebar";
const MIN_WIDTH: u16 = 25;
const MAX_WIDTH: u16 = 50;

/// Global tmux options set while the sidebar is active.
const SIDEBAR_GLOBAL_OPTIONS: &[&str] = &[
    "@workmux_sidebar_enabled",
    "@workmux_sidebar_agents",
    "@workmux_sleeping_panes",
    "@workmux_sidebar_scope",
];

/// Active sidebar scope on this tmux server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SidebarScope {
    /// No sidebar active.
    Off,
    /// Sidebar active in all sessions.
    Global,
    /// Sidebar active in a single session (by stable session_id like "$0").
    Session(String),
}

/// Read the current sidebar scope from tmux.
pub(super) fn current_scope() -> SidebarScope {
    let raw = Cmd::new("tmux")
        .args(&["show-option", "-gqv", "@workmux_sidebar_scope"])
        .run_and_capture_stdout()
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    match raw.as_str() {
        "" => SidebarScope::Off,
        "global" => SidebarScope::Global,
        session_id => SidebarScope::Session(session_id.to_string()),
    }
}

/// Set the sidebar scope in tmux.
fn set_scope(scope: &SidebarScope) {
    match scope {
        SidebarScope::Off => {
            let _ = Cmd::new("tmux")
                .args(&["set-option", "-gu", "@workmux_sidebar_scope"])
                .run();
        }
        SidebarScope::Global => {
            let _ = Cmd::new("tmux")
                .args(&["set-option", "-g", "@workmux_sidebar_scope", "global"])
                .run();
        }
        SidebarScope::Session(id) => {
            let _ = Cmd::new("tmux")
                .args(&["set-option", "-g", "@workmux_sidebar_scope", id])
                .run();
        }
    }
}

/// Get the current tmux session's stable ID (e.g., "$0").
fn get_current_session_id() -> Result<String> {
    let s = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{session_id}"])
        .run_and_capture_stdout()?
        .trim()
        .to_string();
    if s.is_empty() {
        return Err(anyhow!("could not detect tmux session"));
    }
    Ok(s)
}

/// Get the session_id a window belongs to.
fn get_window_session_id(window_id: &str) -> Option<String> {
    Cmd::new("tmux")
        .args(&["display-message", "-t", window_id, "-p", "#{session_id}"])
        .run_and_capture_stdout()
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Unset all sidebar global tmux options.
fn clear_sidebar_globals() {
    for opt in SIDEBAR_GLOBAL_OPTIONS {
        let _ = Cmd::new("tmux").args(&["set-option", "-gu", opt]).run();
    }
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

    if std::env::var("TMUX").is_err() {
        return Err(anyhow!("Sidebar requires tmux"));
    }

    // Can't enable global while session-scoped is active
    if let SidebarScope::Session(_) = current_scope() {
        return Err(anyhow!(
            "Session-scoped sidebar is active. Run `workmux sidebar --session` in that session to disable it first."
        ));
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
    Cmd::new("tmux")
        .args(&["set-option", "-g", "@workmux_sidebar_enabled", "1"])
        .run()?;
    set_scope(&SidebarScope::Global);

    // Ensure daemon is running (spawns if needed)
    ensure_daemon_running()?;

    create_sidebars_in_all_windows(&config)?;
    install_hooks()?;

    Ok(())
}

/// Toggle the sidebar for the current tmux session only.
pub fn toggle_session() -> Result<()> {
    let config = crate::config::Config::load(None)?;

    if std::env::var("TMUX").is_err() {
        return Err(anyhow!("Sidebar requires tmux"));
    }

    let scope = current_scope();
    let session_id = get_current_session_id()?;

    match &scope {
        SidebarScope::Global => {
            return Err(anyhow!(
                "Global sidebar is active. Run `workmux sidebar` to disable it first."
            ));
        }
        SidebarScope::Session(id) if *id != session_id => {
            return Err(anyhow!(
                "Another session already has sidebar active. Disable it first."
            ));
        }
        _ => {}
    }

    let current_window = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{window_id}"])
        .run_and_capture_stdout()?
        .trim()
        .to_string();

    let current_has_sidebar = find_sidebar_in_window(&current_window).unwrap_or(false);

    if current_has_sidebar {
        // Toggle OFF for this session
        kill_sidebars_in_session(&session_id);
        kill_daemon();
        remove_hooks();
        clear_sidebar_globals();
        return Ok(());
    }

    let _ = std::thread::spawn(crate::tips::mark_sidebar_used);

    Cmd::new("tmux")
        .args(&["set-option", "-g", "@workmux_sidebar_enabled", "1"])
        .run()?;
    set_scope(&SidebarScope::Session(session_id.clone()));

    ensure_daemon_running()?;
    create_sidebars_in_session(&session_id, &config)?;
    install_hooks()?;

    Ok(())
}

/// Resolve window ID from an optional argument, falling back to current window.
fn resolve_target_window(window_id: Option<&str>) -> Result<String> {
    match window_id {
        Some(id) => Ok(id.to_string()),
        None => Ok(Cmd::new("tmux")
            .args(&["display-message", "-p", "#{window_id}"])
            .run_and_capture_stdout()?
            .trim()
            .to_string()),
    }
}

/// Sync sidebar into a window (called by tmux hooks for new windows/sessions).
pub fn sync(window_id: Option<&str>) -> Result<()> {
    let scope = current_scope();
    if matches!(scope, SidebarScope::Off) {
        return Ok(());
    }

    // Ensure daemon is running (may have auto-exited or crashed)
    let _ = ensure_daemon_running();

    let target = resolve_target_window(window_id)?;
    if target.is_empty() {
        return Ok(());
    }

    // Session filter: skip windows not in the scoped session
    if let SidebarScope::Session(ref sid) = scope
        && let Some(window_sid) = get_window_session_id(&target)
        && window_sid != *sid
    {
        return Ok(());
    }

    // Check if this window already has a sidebar
    if find_sidebar_in_window(&target)? {
        return Ok(());
    }

    // Compute sidebar width based on the target window's own width, not the
    // stored global. The global is set once at toggle-time and may reflect a
    // different client/terminal size than this window actually has.
    let config = crate::config::Config::load(None).unwrap_or_default();
    let window_w: u16 = Cmd::new("tmux")
        .args(&["display-message", "-t", &target, "-p", "#{window_width}"])
        .run_and_capture_stdout()
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    let width = resolve_width_for(&config, window_w);
    create_sidebar_in_window(&target, width)?;

    Ok(())
}

/// Reflow sidebar layout after a window resize (called by tmux hook).
///
/// Finds the sidebar pane in the target window and runs the layout tree
/// reflow to keep the sidebar at the correct width and content panes balanced.
pub fn reflow(window_id: Option<&str>) -> Result<()> {
    let scope = current_scope();
    if matches!(scope, SidebarScope::Off) {
        return Ok(());
    }

    let target = resolve_target_window(window_id)?;
    if target.is_empty() {
        return Ok(());
    }

    // Session filter: skip windows not in the scoped session
    if let SidebarScope::Session(ref sid) = scope
        && let Some(window_sid) = get_window_session_id(&target)
        && window_sid != *sid
    {
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
