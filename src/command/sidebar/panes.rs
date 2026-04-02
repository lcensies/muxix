//! Sidebar pane creation, destruction, and lifecycle management.

use anyhow::{Result, anyhow};
use tracing::debug;

use crate::cmd::Cmd;

use super::SIDEBAR_ROLE_VALUE;
use super::daemon_ctrl::kill_daemon;
use super::hooks::remove_hooks;
use super::layout_tree::{layout_after_sidebar_remove, reflow_after_sidebar_add};

/// Check if a window already has a sidebar pane.
pub(super) fn find_sidebar_in_window(window_id: &str) -> Result<bool> {
    let output = Cmd::new("tmux")
        .args(&["list-panes", "-t", window_id, "-F", "#{@workmux_role}"])
        .run_and_capture_stdout()?;

    Ok(output.lines().any(|l| l.trim() == SIDEBAR_ROLE_VALUE))
}

/// Create a sidebar pane in a specific window (idempotent).
pub(super) fn create_sidebar_in_window(window_id: &str, width: u16) -> Result<()> {
    if find_sidebar_in_window(window_id).unwrap_or(false) {
        debug!(
            window_id,
            "create_sidebar_in_window: already exists, skipping"
        );
        return Ok(());
    }

    let exe = std::env::current_exe()?;
    let exe_str = exe.to_str().ok_or_else(|| anyhow!("exe path not UTF-8"))?;
    let width_str = width.to_string();

    debug!(window_id, width, "create_sidebar_in_window: creating");

    // Get the first pane in the window as split target
    let target_pane = Cmd::new("tmux")
        .args(&["list-panes", "-t", window_id, "-F", "#{pane_id}"])
        .run_and_capture_stdout()?;
    let target_pane = target_pane.lines().next().map(|l| l.trim()).unwrap_or("");
    if target_pane.is_empty() {
        return Ok(());
    }

    let new_pane_id = Cmd::new("tmux")
        .args(&[
            "split-window",
            "-hbf",
            "-l",
            &width_str,
            "-t",
            target_pane,
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            exe_str,
            "_sidebar-run",
        ])
        .run_and_capture_stdout()?
        .trim()
        .to_string();

    Cmd::new("tmux")
        .args(&[
            "set-option",
            "-p",
            "-t",
            &new_pane_id,
            "@workmux_role",
            SIDEBAR_ROLE_VALUE,
        ])
        .run()?;

    // Reflow the layout tree so content panes share the remaining width
    // proportionally. This is an atomic select-layout operation that avoids
    // the lopsided splits caused by split-window stealing from one pane only.
    reflow_after_sidebar_add(window_id, &new_pane_id, width);

    debug!(
        window_id,
        pane_id = new_pane_id.as_str(),
        requested_width = width,
        "create_sidebar_in_window: done"
    );

    Ok(())
}

/// Create sidebars in all existing tmux windows.
///
/// Computes width per-window from `#{window_width}` so each window gets a
/// sidebar proportional to its own dimensions. Unattached sessions may have
/// stale geometry, but `reflow()` corrects them on reattach.
pub(super) fn create_sidebars_in_all_windows(config: &crate::config::Config) -> Result<()> {
    let output = Cmd::new("tmux")
        .args(&["list-windows", "-a", "-F", "#{window_id} #{window_width}"])
        .run_and_capture_stdout()?;

    debug!("create_sidebars_in_all_windows: creating sidebars");

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (window_id, width_str) = line.split_once(' ').unwrap_or((line, "0"));
        let window_w: u16 = width_str.parse().unwrap_or(0);
        let width = super::resolve_width_for(config, window_w);
        let _ = create_sidebar_in_window(window_id, width);
    }

    Ok(())
}

/// Find all sidebar panes across all windows. Returns (window_id, pane_id) pairs.
fn list_sidebar_panes() -> Vec<(String, String)> {
    let output = Cmd::new("tmux")
        .args(&[
            "list-panes",
            "-a",
            "-F",
            "#{window_id} #{pane_id} #{@workmux_role}",
        ])
        .run_and_capture_stdout()
        .unwrap_or_default();

    output
        .lines()
        .filter_map(|line| {
            let (window_id, rest) = line.split_once(' ')?;
            let (pane_id, role) = rest.split_once(' ')?;
            (role.trim() == SIDEBAR_ROLE_VALUE)
                .then(|| (window_id.to_string(), pane_id.to_string()))
        })
        .collect()
}

/// Kill all sidebar panes and reflow content to fill the window.
///
/// Computes the target layout from the live tree BEFORE killing panes,
/// then applies it after. This preserves pane arrangements the user
/// created while the sidebar was open.
pub(super) fn kill_all_sidebars_and_restore_layouts() {
    let sidebars = list_sidebar_panes();

    // Compute target layouts from the live tree before destroying any panes
    let layouts: Vec<_> = sidebars
        .iter()
        .map(|(window_id, pane_id)| layout_after_sidebar_remove(window_id, pane_id))
        .collect();

    for (_, pane_id) in &sidebars {
        let _ = Cmd::new("tmux").args(&["kill-pane", "-t", pane_id]).run();
    }

    for (i, (window_id, _)) in sidebars.iter().enumerate() {
        if let Some(layout) = &layouts[i] {
            let _ = Cmd::new("tmux")
                .args(&["select-layout", "-t", window_id, layout])
                .run();
        }
    }
}

/// Shut down all sidebars globally (called when any sidebar quits).
/// Kills all other sidebar panes immediately, then defers our own window's
/// layout reflow so it fires after our process exits and the pane closes.
pub(super) fn shutdown_all_sidebars() {
    let our_pane = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{pane_id}"])
        .run_and_capture_stdout()
        .unwrap_or_default()
        .trim()
        .to_string();
    let our_window = Cmd::new("tmux")
        .args(&["display-message", "-p", "#{window_id}"])
        .run_and_capture_stdout()
        .unwrap_or_default()
        .trim()
        .to_string();

    let sidebars = list_sidebar_panes();

    // Compute target layouts from the live tree before destroying any panes
    let computed_layouts: Vec<_> = sidebars
        .iter()
        .map(|(window_id, pane_id)| layout_after_sidebar_remove(window_id, pane_id))
        .collect();

    let mut other_window_layouts = Vec::new();
    let mut our_layout = None;

    for (i, (window_id, pane_id)) in sidebars.iter().enumerate() {
        if pane_id != &our_pane {
            other_window_layouts.push((window_id.clone(), computed_layouts[i].clone()));
            let _ = Cmd::new("tmux").args(&["kill-pane", "-t", pane_id]).run();
        } else {
            our_layout = computed_layouts[i].clone();
        }
    }

    // Apply layouts for other windows
    for (window_id, layout) in &other_window_layouts {
        if let Some(layout) = layout {
            let _ = Cmd::new("tmux")
                .args(&["select-layout", "-t", window_id, layout])
                .run();
        }
    }

    // Kill daemon
    kill_daemon();

    // Remove hooks and global options
    remove_hooks();
    super::clear_sidebar_globals();

    // Defer our own window's layout reflow until after our pane closes
    if !our_window.is_empty()
        && let Some(layout) = our_layout
    {
        let cmd = format!(
            "sleep 0.1; tmux select-layout -t {win} '{layout}' 2>/dev/null",
            win = our_window,
            layout = layout,
        );
        let _ = Cmd::new("tmux").args(&["run-shell", "-b", &cmd]).run();
    }
}
