//! WezTerm backend implementation for the Multiplexer trait.
//!
//! This module provides WezTermBackend, which wraps all WezTerm-specific operations
//! and exposes them through the Multiplexer trait interface.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::cmd::Cmd;
use crate::config::SplitDirection;

use super::agent;
use super::handshake::UnixPipeHandshake;
use super::types::*;
use super::util;
use super::{Multiplexer, PaneHandshake};

/// WezTerm pane information from `wezterm cli list --format json`
#[derive(Debug, Deserialize)]
struct WezTermPane {
    // The following fields are required for JSON deserialization from `wezterm cli list`
    // but are not used after parsing. The allow(dead_code) suppresses false positives.
    #[allow(dead_code)]
    window_id: u64,
    tab_id: u64,
    pane_id: u64,
    workspace: String,
    /// Terminal title (set by running process via escape sequences)
    title: String,
    /// Explicit tab title (we set this for window names)
    tab_title: String,
    /// Working directory in format "file://hostname/path"
    cwd: String,
    #[allow(dead_code)]
    tty_name: Option<String>,
    is_active: bool,
    #[allow(dead_code)]
    is_zoomed: bool,
    #[allow(dead_code)]
    cursor_x: u64,
    #[allow(dead_code)]
    cursor_y: u64,
}

impl WezTermPane {
    /// Parse cwd from "file://hostname/path" format to PathBuf
    fn cwd_path(&self) -> PathBuf {
        // Format: "file://hostname/path" or "file:///path" (empty hostname)
        self.cwd
            .strip_prefix("file://")
            .and_then(|s| {
                // Find first / after hostname
                s.find('/').map(|idx| PathBuf::from(&s[idx..]))
            })
            .unwrap_or_else(|| {
                // Fallback: try parsing as plain path
                PathBuf::from(&self.cwd)
            })
    }
}

/// WezTerm backend implementation.
///
/// Relies on inherited WEZTERM_UNIX_SOCKET and WEZTERM_PANE environment variables.
/// Requires proper WezTerm config (see docs/guide/wezterm.md).
#[derive(Debug)]
pub struct WezTermBackend;

impl Default for WezTermBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WezTermBackend {
    /// Create a new WezTermBackend instance.
    ///
    /// Requires proper WezTerm config:
    /// - `default_gui_startup_args = { 'connect', 'unix' }` in wezterm.lua
    /// - `SpawnTab('CurrentPaneDomain')` for new tab keybindings
    ///
    /// This ensures WEZTERM_UNIX_SOCKET and WEZTERM_PANE are consistent.
    pub fn new() -> Self {
        Self
    }

    /// Create a wezterm CLI command.
    /// Uses inherited WEZTERM_UNIX_SOCKET from environment.
    fn wezterm_cmd(&self) -> Cmd<'static> {
        Cmd::new("wezterm")
    }

    /// Query all panes from WezTerm.
    fn list_panes(&self) -> Result<Vec<WezTermPane>> {
        let output = self
            .wezterm_cmd()
            .args(&["cli", "list", "--format", "json"])
            .run_and_capture_stdout()
            .context("Failed to list WezTerm panes")?;

        let panes: Vec<WezTermPane> =
            serde_json::from_str(&output).context("Failed to parse WezTerm pane list")?;

        Ok(panes)
    }

    /// Get the current workspace name from the environment.
    /// Returns the workspace of the current pane.
    /// Returns None if not running inside WezTerm or if the pane can't be found.
    fn current_workspace(&self) -> Option<String> {
        let pane_id: u64 = std::env::var("WEZTERM_PANE").ok()?.parse().ok()?;
        let panes = self.list_panes().ok()?;
        panes
            .iter()
            .find(|p| p.pane_id == pane_id)
            .map(|p| p.workspace.clone())
    }

    /// Set the tab title for a pane.
    fn set_tab_title(&self, pane_id: &str, title: &str) -> Result<()> {
        self.wezterm_cmd()
            .args(&["cli", "set-tab-title", "--pane-id", pane_id, title])
            .run()
            .context("Failed to set tab title")?;
        Ok(())
    }

    /// Split a pane with optional command.
    fn split_pane_internal(
        &self,
        target_pane_id: &str,
        direction: SplitDirection,
        cwd: &Path,
        size: Option<u16>,
        percentage: Option<u8>,
        command: Option<&str>,
    ) -> Result<String> {
        let direction_arg = match direction {
            SplitDirection::Horizontal => "--horizontal",
            SplitDirection::Vertical => "--top-level",
        };

        let cwd_str = cwd.to_string_lossy();
        let mut args = vec![
            "cli",
            "split-pane",
            "--pane-id",
            target_pane_id,
            "--cwd",
            &*cwd_str,
            direction_arg,
        ];

        let percent_arg;
        if let Some(p) = percentage {
            percent_arg = format!("{}", p);
            args.push("--percent");
            args.push(&percent_arg);
        }
        let _ = size; // WezTerm doesn't support absolute sizes via CLI

        // Handle optional command: always wrap in sh -c to correctly handle
        // both simple commands and complex shell scripts with quoting
        if let Some(cmd) = command {
            args.push("--");
            args.push("sh");
            args.push("-c");
            args.push(cmd);
        }

        let output = self
            .wezterm_cmd()
            .args(&args)
            .run_and_capture_stdout()
            .context("Failed to split WezTerm pane")?;

        Ok(output.trim().to_string())
    }
}

impl Multiplexer for WezTermBackend {
    fn name(&self) -> &'static str {
        "wezterm"
    }

    // === Server/Session ===

    fn is_running(&self) -> Result<bool> {
        self.wezterm_cmd().args(&["cli", "list"]).run_as_check()
    }

    fn current_pane_id(&self) -> Option<String> {
        // WEZTERM_PANE is reliable when WezTerm is properly configured
        // (default_gui_startup_args = { 'connect', 'unix' })
        std::env::var("WEZTERM_PANE").ok()
    }

    fn get_client_active_pane_path(&self) -> Result<PathBuf> {
        let pane_id: u64 = std::env::var("WEZTERM_PANE")
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| anyhow!("WEZTERM_PANE not set or invalid"))?;

        let panes = self.list_panes()?;
        let current = panes
            .iter()
            .find(|p| p.pane_id == pane_id)
            .ok_or_else(|| anyhow!("Current pane {} not found", pane_id))?;

        let path = current.cwd_path();
        if path.as_os_str().is_empty() {
            return Err(anyhow!("Empty path returned from WezTerm"));
        }

        Ok(path)
    }

    // === Window/Tab Management ===

    fn create_window(&self, params: CreateWindowParams) -> Result<String> {
        let full_name = util::prefixed(params.prefix, params.name);
        let cwd_str = params.cwd.to_string_lossy();

        // Note: WezTerm doesn't support "insert after" - tabs appear at end
        // params.after_window is ignored (different from tmux)
        // spawn without --new-window creates a new tab in the current window
        let output = self
            .wezterm_cmd()
            .args(&["cli", "spawn", "--cwd", &*cwd_str])
            .run_and_capture_stdout()
            .context("Failed to create WezTerm tab")?;

        let pane_id = output.trim().to_string();

        // CRITICAL: Set tab_title for persistent window naming
        self.set_tab_title(&pane_id, &full_name)?;

        Ok(pane_id)
    }

    fn create_session(&self, _params: CreateSessionParams) -> Result<String> {
        Err(anyhow!(
            "Session mode (--session) is not supported in WezTerm.\n\
             WezTerm workspaces work differently from tmux sessions.\n\
             Use the default window mode instead (omit --session flag)."
        ))
    }

    fn switch_to_session(&self, _prefix: &str, _name: &str) -> Result<()> {
        Err(anyhow!(
            "Session mode is not supported in WezTerm.\n\
             Use the default window mode instead."
        ))
    }

    fn session_exists(&self, _full_name: &str) -> Result<bool> {
        // WezTerm doesn't have persistent sessions like tmux.
        // Workspaces are ephemeral and not queryable via CLI.
        Ok(false)
    }

    fn kill_session(&self, _full_name: &str) -> Result<()> {
        // WezTerm doesn't have persistent sessions to kill.
        // Workspaces disappear when their last window closes.
        Ok(())
    }

    fn kill_window(&self, full_name: &str) -> Result<()> {
        let panes = self.list_panes()?;
        let current_ws = self.current_workspace();

        // Find all panes in tabs matching this tab_title (filtered by workspace)
        let tab_panes: Vec<_> = panes
            .iter()
            .filter(|p| {
                p.tab_title == full_name && current_ws.as_ref().is_none_or(|ws| &p.workspace == ws)
            })
            .collect();

        if tab_panes.is_empty() {
            return Ok(()); // Already gone
        }

        // Kill in reverse order (last pane first)
        for pane in tab_panes.iter().rev() {
            let _ = self
                .wezterm_cmd()
                .args(&["cli", "kill-pane", "--pane-id", &pane.pane_id.to_string()])
                .run();
        }
        Ok(())
    }

    fn schedule_window_close(&self, full_name: &str, delay: Duration) -> Result<()> {
        let panes = self.list_panes()?;
        let current_ws = self.current_workspace();

        let tab_panes: Vec<_> = panes
            .iter()
            .filter(|p| {
                p.tab_title == full_name && current_ws.as_ref().is_none_or(|ws| &p.workspace == ws)
            })
            .collect();

        if tab_panes.is_empty() {
            return Ok(());
        }

        // Build kill commands for all panes (reverse order)
        let kill_cmds: String = tab_panes
            .iter()
            .rev()
            .map(|p| format!("wezterm cli kill-pane --pane-id {}", p.pane_id))
            .collect::<Vec<_>>()
            .join("; ");

        // nohup inherits WEZTERM_UNIX_SOCKET from environment
        let script = format!(
            "nohup sh -c 'sleep {}; {}' >/dev/null 2>&1 &",
            delay.as_secs_f64(),
            kill_cmds
        );

        Cmd::new("sh").args(&["-c", &script]).run()?;
        Ok(())
    }

    fn schedule_session_close(&self, _full_name: &str, _delay: Duration) -> Result<()> {
        Err(anyhow::anyhow!(
            "Session mode is not supported in WezTerm. Use window mode instead."
        ))
    }

    fn run_deferred_script(&self, script: &str) -> Result<()> {
        // Run the script in the background using nohup
        let bg_script = format!("nohup sh -c '{}' >/dev/null 2>&1 &", script);
        Cmd::new("sh").args(&["-c", &bg_script]).run()?;
        Ok(())
    }

    fn shell_select_window_cmd(&self, full_name: &str) -> Result<String> {
        let panes = self.list_panes()?;
        let current_ws = self.current_workspace();
        let target = panes
            .iter()
            .find(|p| {
                p.tab_title == full_name && current_ws.as_ref().is_none_or(|ws| &p.workspace == ws)
            })
            .ok_or_else(|| anyhow!("Window '{}' not found", full_name))?;
        Ok(format!(
            "wezterm cli activate-tab --tab-id {} >/dev/null 2>&1",
            target.tab_id
        ))
    }

    fn shell_kill_window_cmd(&self, full_name: &str) -> Result<String> {
        let panes = self.list_panes()?;
        let current_ws = self.current_workspace();
        let tab_panes: Vec<_> = panes
            .iter()
            .filter(|p| {
                p.tab_title == full_name && current_ws.as_ref().is_none_or(|ws| &p.workspace == ws)
            })
            .collect();

        if tab_panes.is_empty() {
            return Err(anyhow!("Window '{}' not found", full_name));
        }

        let kill_cmds: String = tab_panes
            .iter()
            .rev()
            .map(|p| {
                format!(
                    "wezterm cli kill-pane --pane-id {} >/dev/null 2>&1",
                    p.pane_id
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        Ok(kill_cmds)
    }

    fn shell_switch_session_cmd(&self, _full_name: &str) -> Result<String> {
        Err(anyhow!(
            "Session mode is not supported in WezTerm. Use window mode instead."
        ))
    }

    fn shell_kill_session_cmd(&self, _full_name: &str) -> Result<String> {
        Err(anyhow!(
            "Session mode is not supported in WezTerm. Use window mode instead."
        ))
    }

    fn select_window(&self, prefix: &str, name: &str) -> Result<()> {
        let full_name = util::prefixed(prefix, name);
        let panes = self.list_panes()?;
        let current_ws = self.current_workspace();

        // Find tab by tab_title (filtered by workspace)
        let target = panes
            .iter()
            .find(|p| {
                p.tab_title == full_name && current_ws.as_ref().is_none_or(|ws| &p.workspace == ws)
            })
            .ok_or_else(|| anyhow!("Window '{}' not found", full_name))?;

        self.wezterm_cmd()
            .args(&[
                "cli",
                "activate-tab",
                "--tab-id",
                &target.tab_id.to_string(),
            ])
            .run()
            .context("Failed to activate tab")?;
        Ok(())
    }

    fn window_exists(&self, prefix: &str, name: &str) -> Result<bool> {
        let full_name = util::prefixed(prefix, name);
        self.window_exists_by_full_name(&full_name)
    }

    fn window_exists_by_full_name(&self, full_name: &str) -> Result<bool> {
        let names = self.get_all_window_names()?;
        Ok(names.contains(full_name))
    }

    fn current_window_name(&self) -> Result<Option<String>> {
        let pane_id: u64 = match std::env::var("WEZTERM_PANE")
            .ok()
            .and_then(|s| s.parse().ok())
        {
            Some(id) => id,
            None => return Ok(None),
        };

        let panes = self.list_panes()?;
        let current = panes.iter().find(|p| p.pane_id == pane_id);

        Ok(current.map(|p| p.tab_title.clone()))
    }

    fn get_all_window_names(&self) -> Result<HashSet<String>> {
        let panes = self.list_panes()?;
        let current_ws = self.current_workspace();

        // Collect unique tab_titles (our window names), filtered by current workspace
        // If we can't determine the workspace (not in WezTerm), show all
        let names: HashSet<String> = panes
            .iter()
            .filter(|p| current_ws.as_ref().is_none_or(|ws| &p.workspace == ws))
            .map(|p| p.tab_title.clone())
            .collect();

        Ok(names)
    }

    fn get_all_session_names(&self) -> Result<HashSet<String>> {
        // WezTerm doesn't support session mode - return empty set
        Ok(HashSet::new())
    }

    fn filter_active_windows(&self, windows: &[String]) -> Result<Vec<String>> {
        let all_current = self.get_all_window_names()?;

        Ok(windows
            .iter()
            .filter(|w| all_current.contains(*w))
            .cloned()
            .collect())
    }

    fn find_last_window_with_prefix(&self, _prefix: &str) -> Result<Option<String>> {
        // WezTerm doesn't support tab insertion ordering via CLI
        // Return None - new tabs will appear at end
        Ok(None)
    }

    fn find_last_window_with_base_handle(
        &self,
        _prefix: &str,
        _base_handle: &str,
    ) -> Result<Option<String>> {
        // WezTerm doesn't support tab insertion ordering via CLI
        Ok(None)
    }

    fn wait_until_windows_closed(&self, full_window_names: &[String]) -> Result<()> {
        if full_window_names.is_empty() {
            return Ok(());
        }

        let targets: HashSet<String> = full_window_names.iter().cloned().collect();

        if targets.len() == 1 {
            println!("Waiting for window '{}' to close...", full_window_names[0]);
        } else {
            println!("Waiting for {} windows to close...", targets.len());
        }

        loop {
            if !self.is_running()? {
                return Ok(());
            }

            let current_windows = self.get_all_window_names()?;

            let any_exists = targets
                .iter()
                .any(|target| current_windows.contains(target));

            if !any_exists {
                return Ok(());
            }

            thread::sleep(Duration::from_millis(500));
        }
    }

    fn wait_until_session_closed(&self, _full_session_name: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "Session mode is not supported in WezTerm. Use window mode instead."
        ))
    }

    // === Pane Management ===

    fn select_pane(&self, pane_id: &str) -> Result<()> {
        self.wezterm_cmd()
            .args(&["cli", "activate-pane", "--pane-id", pane_id])
            .run()
            .context("Failed to select pane")?;
        Ok(())
    }

    fn switch_to_pane(&self, pane_id: &str) -> Result<()> {
        // Check if we need to switch workspaces first
        let panes = self.list_panes()?;
        if let Some(target) = panes.iter().find(|p| p.pane_id.to_string() == pane_id) {
            let target_workspace = &target.workspace;
            if let Some(current) = self.current_workspace()
                && &current != target_workspace
            {
                // Cross-workspace switch: send escape sequence to trigger Lua handler
                // Use tab_title (stable across mux contexts) instead of pane_id
                send_pane_switch_signal(target_workspace, &target.tab_title);
                return Ok(());
            }
        }

        // Same workspace: use CLI directly
        self.select_pane(pane_id)
    }

    fn respawn_pane(&self, pane_id: &str, cwd: &Path, cmd: Option<&str>) -> Result<String> {
        let panes = self.list_panes()?;
        let target = panes
            .iter()
            .find(|p| p.pane_id.to_string() == pane_id)
            .ok_or_else(|| anyhow!("Pane {} not found", pane_id))?;

        let tab_id = target.tab_id;
        let original_tab_title = target.tab_title.clone();

        // Find a sibling pane in the same tab (to split from after kill)
        let sibling = panes
            .iter()
            .find(|p| p.tab_id == tab_id && p.pane_id.to_string() != pane_id);

        if let Some(sib) = sibling {
            // Has sibling: kill target, split from sibling
            self.wezterm_cmd()
                .args(&["cli", "kill-pane", "--pane-id", pane_id])
                .run()?;

            let new_pane_id = self.split_pane_internal(
                &sib.pane_id.to_string(),
                SplitDirection::Horizontal,
                cwd,
                None,
                None,
                cmd,
            )?;

            Ok(new_pane_id)
        } else {
            // Only pane in tab: spawn new tab, kill old
            let cwd_str = cwd.to_string_lossy();
            let mut args = vec!["cli", "spawn", "--cwd", &*cwd_str];

            // Wrap in sh -c to correctly handle complex shell scripts with quoting
            if let Some(c) = cmd {
                args.push("--");
                args.push("sh");
                args.push("-c");
                args.push(c);
            }

            let output = self
                .wezterm_cmd()
                .args(&args)
                .run_and_capture_stdout()
                .context("Failed to spawn new tab")?;

            let new_pane_id = output.trim().to_string();

            // Set tab title to preserve window name
            self.set_tab_title(&new_pane_id, &original_tab_title)?;

            // Kill old pane (tab will close but new tab exists)
            let _ = self
                .wezterm_cmd()
                .args(&["cli", "kill-pane", "--pane-id", pane_id])
                .run();

            Ok(new_pane_id)
        }
    }

    fn capture_pane(&self, pane_id: &str, lines: u16) -> Option<String> {
        // Note: We don't use --escapes to avoid partial escape sequences like (B
        // appearing in the preview. Plain text is cleaner for dashboard display.
        let output = self
            .wezterm_cmd()
            .args(&["cli", "get-text", "--pane-id", pane_id])
            .run_and_capture_stdout()
            .ok()?;

        // get-text returns all scrollback; take last N lines
        let all_lines: Vec<&str> = output.lines().collect();
        let start = all_lines.len().saturating_sub(lines as usize);
        Some(all_lines[start..].join("\n"))
    }

    // === Text I/O ===

    fn send_keys(&self, pane_id: &str, command: &str) -> Result<()> {
        // WezTerm send-text sends literal text
        self.wezterm_cmd()
            .args(&[
                "cli",
                "send-text",
                "--pane-id",
                pane_id,
                "--no-paste",
                command,
            ])
            .run()?;

        // Send Enter
        self.wezterm_cmd()
            .args(&["cli", "send-text", "--pane-id", pane_id, "--no-paste", "\r"])
            .run()?;

        Ok(())
    }

    fn send_keys_to_agent(&self, pane_id: &str, command: &str, agent: Option<&str>) -> Result<()> {
        if agent::resolve_profile(agent).needs_bang_delay() && command.starts_with('!') {
            // Send ! first
            self.wezterm_cmd()
                .args(&["cli", "send-text", "--pane-id", pane_id, "--no-paste", "!"])
                .run()
                .context("Failed to send ! to pane")?;

            // Small delay to let Claude register the !
            thread::sleep(Duration::from_millis(50));

            // Send the rest of the command (without the !)
            self.wezterm_cmd()
                .args(&[
                    "cli",
                    "send-text",
                    "--pane-id",
                    pane_id,
                    "--no-paste",
                    &command[1..],
                ])
                .run()
                .context("Failed to send keys to pane")?;

            // Send Enter
            self.wezterm_cmd()
                .args(&["cli", "send-text", "--pane-id", pane_id, "--no-paste", "\r"])
                .run()
                .context("Failed to send Enter key to pane")?;

            Ok(())
        } else {
            self.send_keys(pane_id, command)
        }
    }

    fn send_key(&self, pane_id: &str, key: &str) -> Result<()> {
        self.wezterm_cmd()
            .args(&["cli", "send-text", "--pane-id", pane_id, "--no-paste", key])
            .run()
            .context("Failed to send key to pane")?;
        Ok(())
    }

    fn paste_multiline(&self, pane_id: &str, content: &str) -> Result<()> {
        // Without --no-paste, WezTerm uses bracketed paste
        self.wezterm_cmd()
            .args(&["cli", "send-text", "--pane-id", pane_id, content])
            .run()?;

        // Send Enter to submit
        self.wezterm_cmd()
            .args(&["cli", "send-text", "--pane-id", pane_id, "--no-paste", "\r"])
            .run()?;

        Ok(())
    }

    // === Shell ===

    fn get_default_shell(&self) -> Result<String> {
        // WezTerm doesn't have a config query CLI
        // Use $SHELL or fall back to /bin/bash
        std::env::var("SHELL").or_else(|_| Ok("/bin/bash".to_string()))
    }

    fn create_handshake(&self) -> Result<Box<dyn PaneHandshake>> {
        Ok(Box::new(UnixPipeHandshake::new()?))
    }

    // === Status ===

    fn set_status(&self, pane_id: &str, icon: &str, _auto_clear_on_focus: bool) -> Result<()> {
        // For WezTerm, we could update the tab title to include the icon.
        // However, agent state is now managed by StateStore, so this is just UI feedback.
        // For now, we just log the status change - tab title remains stable.
        // Future: could update tab title to show icon like "🔄 wm-feature"
        let _ = (pane_id, icon); // Acknowledge parameters
        Ok(())
    }

    fn clear_status(&self, _pane_id: &str) -> Result<()> {
        // No UI cleanup needed - tab title remains stable
        Ok(())
    }

    fn ensure_status_format(&self, _pane_id: &str) -> Result<()> {
        // No-op for WezTerm - status is displayed via tab title, not tmux-style format
        Ok(())
    }

    // === Multi-Session/Workspace Support ===

    fn current_session(&self) -> Option<String> {
        self.current_workspace()
    }

    // === State Reconciliation ===

    fn instance_id(&self) -> String {
        // Use the unix socket path as instance ID so all workspaces on the same
        // WezTerm server share one instance — matching tmux behavior where all
        // sessions on the same server are visible in the dashboard.
        std::env::var("WEZTERM_UNIX_SOCKET").unwrap_or_else(|_| "default".to_string())
    }

    fn active_pane_id(&self) -> Option<String> {
        // Query WezTerm for the active pane
        self.list_panes().ok().and_then(|panes| {
            panes
                .into_iter()
                .find(|p| p.is_active)
                .map(|p| p.pane_id.to_string())
        })
    }

    fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>> {
        let pane_id_num: u64 = pane_id.parse().ok().unwrap_or(0);

        let panes = self.list_panes()?;
        let pane = panes.into_iter().find(|p| p.pane_id == pane_id_num);

        match pane {
            Some(p) => {
                // WezTerm doesn't expose PID or current command via CLI list.
                // We extract both from the TTY using ps.
                // TODO: Consider using sysinfo crate instead of shelling out to ps.
                // This would be faster (no subprocess) and more portable (ps flags
                // differ between BSD/Linux).
                let tty_name = p.tty_name.as_ref().map(|t| t.trim_start_matches("/dev/"));

                // Get foreground process PID (process with '+' in STAT)
                // This matches what we store in state files (the agent PID, not shell PID)
                let pid = tty_name
                    .and_then(|tty| {
                        Cmd::new("sh")
                            .args(&[
                                "-c",
                                &format!(
                                    "ps -t {} -o pid=,stat= | grep '+' | head -1 | awk '{{print $1}}'",
                                    tty
                                ),
                            ])
                            .run_and_capture_stdout()
                            .ok()
                    })
                    .and_then(|output| output.trim().parse::<u32>().ok())
                    .unwrap_or(0);

                // Get foreground command (process with '+' in STAT indicates foreground)
                // This is the actual running command (e.g., "claude", "vim", "zsh")
                let current_command = tty_name
                    .and_then(|tty| {
                        // Find the foreground process group and get its command
                        // ps -t <tty> -o stat=,comm= shows STAT and COMMAND
                        // Foreground processes have '+' in STAT (e.g., "S+", "R+")
                        Cmd::new("sh")
                            .args(&[
                                "-c",
                                &format!(
                                    "ps -t {} -o stat=,comm= | grep '+' | head -1 | awk '{{print $2}}'",
                                    tty
                                ),
                            ])
                            .run_and_capture_stdout()
                            .ok()
                    })
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "unknown".to_string());

                Ok(Some(LivePaneInfo {
                    pid,
                    current_command,
                    working_dir: p.cwd_path(),
                    title: if p.title.is_empty() {
                        None
                    } else {
                        Some(p.title.clone())
                    },
                    session: Some(p.workspace.clone()),
                    window: Some(p.tab_title.clone()),
                }))
            }
            None => Ok(None),
        }
    }

    fn get_all_window_names_all_sessions(&self) -> Result<HashSet<String>> {
        // `wezterm cli list` returns ALL panes across ALL workspaces.
        // Just collect unique tab_titles.
        let panes = self.list_panes()?;
        let names: HashSet<String> = panes.iter().map(|p| p.tab_title.clone()).collect();
        Ok(names)
    }

    fn get_all_live_pane_info(&self) -> Result<std::collections::HashMap<String, LivePaneInfo>> {
        use std::collections::HashMap;

        let mut result = HashMap::new();

        for p in self.list_panes()? {
            let pane_id = p.pane_id.to_string();

            // WezTerm doesn't expose PID or current command via CLI list.
            // We extract both from the TTY using ps.
            let tty_name = p.tty_name.as_ref().map(|t| t.trim_start_matches("/dev/"));

            let pid = tty_name
                .and_then(|tty| {
                    Cmd::new("sh")
                        .args(&[
                            "-c",
                            &format!(
                                "ps -t {} -o pid=,stat= | grep '+' | head -1 | awk '{{print $1}}'",
                                tty
                            ),
                        ])
                        .run_and_capture_stdout()
                        .ok()
                })
                .and_then(|output| output.trim().parse::<u32>().ok())
                .unwrap_or(0);

            let current_command = tty_name
                .and_then(|tty| {
                    Cmd::new("sh")
                        .args(&[
                            "-c",
                            &format!(
                                "ps -t {} -o stat=,comm= | grep '+' | head -1 | awk '{{print $2}}'",
                                tty
                            ),
                        ])
                        .run_and_capture_stdout()
                        .ok()
                })
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "unknown".to_string());

            result.insert(
                pane_id,
                LivePaneInfo {
                    pid,
                    current_command,
                    working_dir: p.cwd_path(),
                    title: if p.title.is_empty() {
                        None
                    } else {
                        Some(p.title.clone())
                    },
                    session: Some(p.workspace.clone()),
                    window: Some(p.tab_title.clone()),
                },
            );
        }

        Ok(result)
    }

    fn split_pane(
        &self,
        target_pane_id: &str,
        direction: &SplitDirection,
        cwd: &Path,
        size: Option<u16>,
        percentage: Option<u8>,
        command: Option<&str>,
    ) -> Result<String> {
        self.split_pane_internal(
            target_pane_id,
            direction.clone(),
            cwd,
            size,
            percentage,
            command,
        )
    }
}

/// Send escape sequence to trigger cross-workspace pane switch via WezTerm's user-var-changed event.
///
/// This requires the user to have a Lua handler in their wezterm.lua.
/// The value is a JSON payload with workspace and tab_title.
/// See docs/guide/wezterm.md for the required handler.
///
/// Without this handler, the escape sequence is silently ignored.
fn send_pane_switch_signal(workspace: &str, tab_title: &str) {
    use base64::Engine;
    use std::io::Write;

    // Send JSON with workspace and tab_title (stable across mux contexts)
    let payload = serde_json::json!({
        "workspace": workspace,
        "tab_title": tab_title
    });
    let encoded = base64::engine::general_purpose::STANDARD.encode(payload.to_string());
    // OSC 1337 ; SetUserVar=name=base64_value BEL
    print!("\x1b]1337;SetUserVar=workmux-switch-pane={}\x07", encoded);
    // Flush to ensure it's sent immediately
    let _ = std::io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cwd_path_parsing() {
        let pane = WezTermPane {
            window_id: 0,
            tab_id: 0,
            pane_id: 0,
            workspace: "default".to_string(),
            title: "".to_string(),
            tab_title: "test".to_string(),
            cwd: "file://hostname/home/user/project".to_string(),
            tty_name: None,
            is_active: true,
            is_zoomed: false,
            cursor_x: 0,
            cursor_y: 0,
        };

        assert_eq!(pane.cwd_path(), PathBuf::from("/home/user/project"));
    }

    #[test]
    fn test_cwd_path_parsing_empty_hostname() {
        let pane = WezTermPane {
            window_id: 0,
            tab_id: 0,
            pane_id: 0,
            workspace: "default".to_string(),
            title: "".to_string(),
            tab_title: "test".to_string(),
            cwd: "file:///home/user/project".to_string(),
            tty_name: None,
            is_active: true,
            is_zoomed: false,
            cursor_x: 0,
            cursor_y: 0,
        };

        assert_eq!(pane.cwd_path(), PathBuf::from("/home/user/project"));
    }
}
