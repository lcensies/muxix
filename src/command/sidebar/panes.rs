//! Sidebar pane creation, destruction, and lifecycle management.

use anyhow::{Result, anyhow};
use tracing::debug;

use crate::cmd::Cmd;

use super::SIDEBAR_ROLE_VALUE;
use super::daemon_ctrl::kill_daemon;
use super::hooks::remove_hooks;
use super::layout::{restore_window_layout, save_window_layout};
use super::layout_tree::reflow_after_sidebar_add;

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
    save_window_layout(window_id);

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
pub(super) fn create_sidebars_in_all_windows(width: u16) -> Result<()> {
    let output = Cmd::new("tmux")
        .args(&["list-windows", "-a", "-F", "#{window_id}"])
        .run_and_capture_stdout()?;

    debug!(width, "create_sidebars_in_all_windows: creating sidebars");

    // Clear stale saved layouts from previous cycles that could interfere
    // with layout restore on toggle off.
    for window_id in output.lines() {
        let window_id = window_id.trim();
        if !window_id.is_empty() {
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

    for window_id in output.lines() {
        let window_id = window_id.trim();
        if window_id.is_empty() {
            continue;
        }
        // Use client-based width for all windows. Unattached sessions have stale
        // #{window_width} from their last-seen client (window-size=latest), so
        // per-window sizing gives wrong results for those sessions.
        let _ = create_sidebar_in_window(window_id, width);
    }

    Ok(())
}

/// Kill all sidebar panes and restore the original layout in each window.
pub(super) fn kill_all_sidebars_and_restore_layouts() {
    // Find all sidebar panes with their window IDs
    let output = Cmd::new("tmux")
        .args(&[
            "list-panes",
            "-a",
            "-F",
            "#{window_id} #{pane_id} #{@workmux_role}",
        ])
        .run_and_capture_stdout()
        .unwrap_or_default();

    let mut windows_with_sidebars = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() == 3 && parts[2].trim() == SIDEBAR_ROLE_VALUE {
            windows_with_sidebars.push(parts[0].to_string());
            let _ = Cmd::new("tmux").args(&["kill-pane", "-t", parts[1]]).run();
        }
    }

    // Restore saved layouts
    for window_id in &windows_with_sidebars {
        restore_window_layout(window_id);
    }
}

/// Check if the sidebar is the only pane left in its window.
pub(super) fn is_last_pane_in_window() -> bool {
    Cmd::new("tmux")
        .args(&["list-panes", "-F", "#{pane_id}"])
        .run_and_capture_stdout()
        .map(|s| s.lines().count() <= 1)
        .unwrap_or(false)
}

/// Shut down all sidebars globally (called when any sidebar quits).
/// Kills all other sidebar panes immediately, then defers our own window's
/// layout restore so it fires after our process exits and the pane closes.
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

    let output = Cmd::new("tmux")
        .args(&[
            "list-panes",
            "-a",
            "-F",
            "#{window_id} #{pane_id} #{@workmux_role}",
        ])
        .run_and_capture_stdout()
        .unwrap_or_default();

    let mut other_windows = Vec::new();

    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() == 3 && parts[2].trim() == SIDEBAR_ROLE_VALUE {
            let pane_id = parts[1].trim();
            if pane_id != our_pane {
                other_windows.push(parts[0].to_string());
                let _ = Cmd::new("tmux").args(&["kill-pane", "-t", pane_id]).run();
            }
        }
    }

    // Restore layouts for other windows
    for window_id in &other_windows {
        restore_window_layout(window_id);
    }

    // Kill daemon
    kill_daemon();

    // Remove hooks and global options
    remove_hooks();
    super::clear_sidebar_globals();

    // Defer our own window's layout restore until after our pane closes
    if !our_window.is_empty()
        && let Ok(layout) = Cmd::new("tmux")
            .args(&[
                "show-option",
                "-wqv",
                "-t",
                &our_window,
                "@workmux_sidebar_layout",
            ])
            .run_and_capture_stdout()
    {
        let layout = layout.trim().to_string();
        if !layout.is_empty() {
            let cmd = format!(
                "sleep 0.1 && tmux select-layout -t {win} '{layout}' && tmux set-option -wu -t {win} @workmux_sidebar_layout",
                win = our_window,
                layout = layout,
            );
            let _ = Cmd::new("tmux").args(&["run-shell", "-b", &cmd]).run();
        }
    }
}
