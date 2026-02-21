//! Kitty backend implementation for the Multiplexer trait.
//!
//! This module provides KittyBackend, which wraps all kitty-specific operations
//! and exposes them through the Multiplexer trait interface.
//!
//! Note on terminology:
//! - Kitty "window" = workmux "pane" (a terminal split)
//! - Kitty "tab" = workmux "window" (a named tab)
//! - Kitty "OS window" = the actual window on screen

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
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

/// Kitty process info from `foreground_processes` in ls output
#[derive(Debug, Deserialize)]
struct KittyProcess {
    pid: u32,
    #[allow(dead_code)]
    cwd: String,
    cmdline: Vec<String>,
}

/// Kitty window (= workmux pane) from `kitten @ ls`
#[derive(Debug, Deserialize)]
struct KittyWindow {
    id: u64,
    title: String,
    cwd: String,
    pid: u32,
    is_focused: bool,
    #[allow(dead_code)]
    is_active: bool,
    #[serde(default)]
    foreground_processes: Vec<KittyProcess>,
}

/// Kitty tab (= workmux window) from `kitten @ ls`
#[derive(Debug, Deserialize)]
struct KittyTab {
    id: u64,
    title: String,
    is_active: bool,
    #[allow(dead_code)]
    is_focused: bool,
    windows: Vec<KittyWindow>,
}

/// Kitty OS window from `kitten @ ls`
#[derive(Debug, Deserialize)]
struct KittyOsWindow {
    id: u64,
    is_focused: bool,
    tabs: Vec<KittyTab>,
}

/// Flattened pane info for internal use
#[derive(Debug, Clone)]
struct FlatPane {
    os_window_id: u64,
    tab_id: u64,
    tab_title: String,
    window_id: u64,
    is_focused: bool,
    #[allow(dead_code)]
    is_tab_active: bool,
    cwd: PathBuf,
    pid: u32,
    title: String,
    foreground_command: Option<String>,
    foreground_pid: Option<u32>,
}

/// Kitty backend implementation.
///
/// Relies on inherited KITTY_WINDOW_ID and KITTY_LISTEN_ON environment variables.
/// Requires kitty configuration with `allow_remote_control yes` and `listen_on`.
#[derive(Debug)]
pub struct KittyBackend;

impl Default for KittyBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl KittyBackend {
    /// Create a new KittyBackend instance.
    pub fn new() -> Self {
        Self
    }

    /// Create a kitten @ command.
    fn kitten_cmd(&self) -> Cmd<'static> {
        Cmd::new("kitten").arg("@")
    }

    /// Query all windows/tabs/panes as flat list.
    fn list_panes(&self) -> Result<Vec<FlatPane>> {
        let output = self
            .kitten_cmd()
            .arg("ls")
            .run_and_capture_stdout()
            .context("Failed to list kitty panes")?;

        let os_windows: Vec<KittyOsWindow> =
            serde_json::from_str(&output).context("Failed to parse kitty ls output")?;

        let mut panes = Vec::new();
        for os_win in os_windows {
            for tab in os_win.tabs {
                for win in tab.windows {
                    // Get foreground process info. Use the process with the lowest
                    // PID, which is the most stable (the original user command like
                    // "claude"), not transient children (like "kitten" or "node").
                    // This matters because set-window-status calls kitten @ ls to
                    // capture foreground info, and without min_by we'd capture the
                    // kitten subprocess itself, causing reconciliation to delete the
                    // agent on the next check.
                    let fg = win.foreground_processes.iter().min_by_key(|p| p.pid);
                    let foreground_command = fg.and_then(|p| {
                        p.cmdline.first().map(|c| {
                            Path::new(c)
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_else(|| c.clone())
                        })
                    });
                    let foreground_pid = fg.map(|p| p.pid);

                    panes.push(FlatPane {
                        os_window_id: os_win.id,
                        tab_id: tab.id,
                        tab_title: tab.title.clone(),
                        window_id: win.id,
                        is_focused: win.is_focused && tab.is_focused && os_win.is_focused,
                        is_tab_active: tab.is_active,
                        cwd: PathBuf::from(&win.cwd),
                        pid: win.pid,
                        title: win.title,
                        foreground_command,
                        foreground_pid,
                    });
                }
            }
        }

        Ok(panes)
    }

    /// Get current window ID from environment.
    fn current_window_id(&self) -> Option<u64> {
        std::env::var("KITTY_WINDOW_ID").ok()?.parse().ok()
    }

    /// Get current OS window ID by looking up the current window.
    fn current_os_window_id(&self) -> Option<u64> {
        let window_id = self.current_window_id()?;
        let panes = self.list_panes().ok()?;
        panes
            .iter()
            .find(|p| p.window_id == window_id)
            .map(|p| p.os_window_id)
    }

    /// Filter panes to current OS window scope.
    fn panes_in_current_scope<'a>(&self, panes: &'a [FlatPane]) -> Vec<&'a FlatPane> {
        let current_os = self.current_os_window_id();
        panes
            .iter()
            .filter(|p| current_os.is_none() || Some(p.os_window_id) == current_os)
            .collect()
    }

    /// Set the tab title for a window.
    #[allow(dead_code)]
    fn set_tab_title(&self, window_id: &str, title: &str) -> Result<()> {
        self.kitten_cmd()
            .args(&[
                "set-tab-title",
                "--match",
                &format!("id:{}", window_id),
                title,
            ])
            .run()
            .context("Failed to set tab title")?;
        Ok(())
    }

    /// Internal split pane implementation.
    fn split_pane_internal(
        &self,
        target_pane_id: &str,
        direction: SplitDirection,
        cwd: &Path,
        _size: Option<u16>,
        _percentage: Option<u8>,
        command: Option<&str>,
    ) -> Result<String> {
        // kitty's naming refers to the split line orientation, opposite of tmux:
        //   hsplit = horizontal divider = top/bottom panes
        //   vsplit = vertical divider   = left/right panes
        let location_arg = match direction {
            SplitDirection::Horizontal => "vsplit",
            SplitDirection::Vertical => "hsplit",
        };

        let cwd_str = cwd.to_string_lossy();
        let match_arg = format!("id:{}", target_pane_id);

        let mut args = vec![
            "launch",
            "--location",
            location_arg,
            "--match",
            &match_arg,
            "--cwd",
            &*cwd_str,
        ];

        // Pass command as separate argv tokens for kitten @ launch
        if let Some(cmd) = command {
            args.push("sh");
            args.push("-c");
            args.push(cmd);
        }

        let output = self
            .kitten_cmd()
            .args(&args)
            .run_and_capture_stdout()
            .context("Failed to split kitty pane")?;

        // kitten @ launch returns the new window ID
        Ok(output.trim().to_string())
    }
}

impl Multiplexer for KittyBackend {
    fn name(&self) -> &'static str {
        "kitty"
    }

    // === Server/Session ===

    fn is_running(&self) -> Result<bool> {
        self.kitten_cmd().arg("ls").run_as_check()
    }

    fn current_pane_id(&self) -> Option<String> {
        std::env::var("KITTY_WINDOW_ID").ok()
    }

    fn active_pane_id(&self) -> Option<String> {
        self.list_panes().ok().and_then(|panes| {
            panes
                .into_iter()
                .find(|p| p.is_focused)
                .map(|p| p.window_id.to_string())
        })
    }

    fn get_client_active_pane_path(&self) -> Result<PathBuf> {
        let window_id = self
            .current_window_id()
            .ok_or_else(|| anyhow!("KITTY_WINDOW_ID not set or invalid"))?;

        let panes = self.list_panes()?;
        let current = panes
            .iter()
            .find(|p| p.window_id == window_id)
            .ok_or_else(|| anyhow!("Current window {} not found", window_id))?;

        if current.cwd.as_os_str().is_empty() {
            return Err(anyhow!("Empty path returned from kitty"));
        }

        Ok(current.cwd.clone())
    }

    // === Session Management (not supported in Kitty) ===

    fn create_session(&self, _params: CreateSessionParams) -> Result<String> {
        Err(anyhow!(
            "Session mode (--session) is not supported in Kitty.\n\
             Kitty does not have a session concept like tmux.\n\
             Use the default window mode instead (omit --session flag)."
        ))
    }

    fn switch_to_session(&self, _prefix: &str, _name: &str) -> Result<()> {
        Err(anyhow!(
            "Session mode is not supported in Kitty.\n\
             Use the default window mode instead."
        ))
    }

    fn session_exists(&self, _full_name: &str) -> Result<bool> {
        Ok(false)
    }

    fn kill_session(&self, _full_name: &str) -> Result<()> {
        Ok(())
    }

    fn schedule_session_close(&self, _full_name: &str, _delay: Duration) -> Result<()> {
        Err(anyhow!(
            "Session mode is not supported in Kitty. Use window mode instead."
        ))
    }

    fn get_all_session_names(&self) -> Result<HashSet<String>> {
        Ok(HashSet::new())
    }

    fn wait_until_session_closed(&self, _full_session_name: &str) -> Result<()> {
        Err(anyhow!(
            "Session mode is not supported in Kitty. Use window mode instead."
        ))
    }

    // === Window/Tab Management ===

    fn create_window(&self, params: CreateWindowParams) -> Result<String> {
        let full_name = util::prefixed(params.prefix, params.name);
        let cwd_str = params.cwd.to_string_lossy();

        // Note: kitty doesn't support "insert after" - tabs appear at end
        // params.after_window is ignored (same as WezTerm)
        let output = self
            .kitten_cmd()
            .args(&[
                "launch",
                "--type=tab",
                "--tab-title",
                &full_name,
                "--cwd",
                &*cwd_str,
                "--dont-take-focus",
            ])
            .run_and_capture_stdout()
            .context("Failed to create kitty tab")?;

        let window_id = output.trim().to_string();

        // Persistently set the tab title. The --tab-title flag on launch gets
        // overridden by kitty's dynamic title updates, but set-tab-title locks it.
        let _ = self
            .kitten_cmd()
            .args(&[
                "set-tab-title",
                "--match",
                &format!("id:{}", window_id),
                &full_name,
            ])
            .run();

        Ok(window_id)
    }

    fn kill_window(&self, full_name: &str) -> Result<()> {
        let panes = self.list_panes()?;
        let scoped_panes = self.panes_in_current_scope(&panes);

        // Use window_id (not tab_id) because kitty's --match id:N resolves
        // window IDs, not tab IDs. One window per tab is enough since
        // close-tab closes the entire tab containing the matched window.
        let mut seen_tabs = HashSet::new();
        let window_ids: Vec<u64> = scoped_panes
            .iter()
            .filter(|p| p.tab_title == full_name)
            .filter(|p| seen_tabs.insert(p.tab_id))
            .map(|p| p.window_id)
            .collect();

        if window_ids.is_empty() {
            return Ok(()); // Already gone
        }

        for window_id in window_ids {
            let _ = self
                .kitten_cmd()
                .args(&["close-tab", "--match", &format!("id:{}", window_id)])
                .run();
        }
        Ok(())
    }

    fn schedule_window_close(&self, full_name: &str, delay: Duration) -> Result<()> {
        let panes = self.list_panes()?;
        let scoped_panes = self.panes_in_current_scope(&panes);

        // Use window_id (not tab_id) because kitty's --match id:N resolves
        // window IDs, not tab IDs.
        let mut seen_tabs = HashSet::new();
        let window_ids: Vec<u64> = scoped_panes
            .iter()
            .filter(|p| p.tab_title == full_name)
            .filter(|p| seen_tabs.insert(p.tab_id))
            .map(|p| p.window_id)
            .collect();

        if window_ids.is_empty() {
            return Ok(());
        }

        // Build close commands for all tabs
        let close_cmds: String = window_ids
            .iter()
            .map(|id| format!("kitten @ close-tab --match 'id:{}'", id))
            .collect::<Vec<_>>()
            .join("; ");

        // Use nohup to run in background
        let script = format!(
            "nohup sh -c 'sleep {}; {}' >/dev/null 2>&1 &",
            delay.as_secs_f64(),
            close_cmds
        );

        Cmd::new("sh").args(&["-c", &script]).run()?;
        Ok(())
    }

    fn run_deferred_script(&self, script: &str) -> Result<()> {
        // Run the script in the background using nohup
        let bg_script = format!("nohup sh -c '{}' >/dev/null 2>&1 &", script);
        Cmd::new("sh").args(&["-c", &bg_script]).run()?;
        Ok(())
    }

    fn shell_select_window_cmd(&self, full_name: &str) -> Result<String> {
        let panes = self.list_panes()?;
        let scoped_panes = self.panes_in_current_scope(&panes);
        let target = scoped_panes
            .iter()
            .find(|p| p.tab_title == full_name)
            .ok_or_else(|| anyhow!("Window '{}' not found", full_name))?;
        Ok(format!(
            "kitten @ focus-tab --match 'id:{}' >/dev/null 2>&1",
            target.window_id
        ))
    }

    fn shell_kill_window_cmd(&self, full_name: &str) -> Result<String> {
        let panes = self.list_panes()?;
        let scoped_panes = self.panes_in_current_scope(&panes);
        let target = scoped_panes
            .iter()
            .find(|p| p.tab_title == full_name)
            .ok_or_else(|| anyhow!("Window '{}' not found", full_name))?;
        Ok(format!(
            "kitten @ close-tab --match 'id:{}' >/dev/null 2>&1",
            target.window_id
        ))
    }

    fn shell_switch_session_cmd(&self, _full_name: &str) -> Result<String> {
        Err(anyhow!(
            "Session mode is not supported in Kitty. Use window mode instead."
        ))
    }

    fn shell_kill_session_cmd(&self, _full_name: &str) -> Result<String> {
        Err(anyhow!(
            "Session mode is not supported in Kitty. Use window mode instead."
        ))
    }

    fn select_window(&self, prefix: &str, name: &str) -> Result<()> {
        let full_name = util::prefixed(prefix, name);
        let panes = self.list_panes()?;
        let scoped_panes = self.panes_in_current_scope(&panes);

        // Find tab by tab_title
        let target = scoped_panes
            .iter()
            .find(|p| p.tab_title == full_name)
            .ok_or_else(|| anyhow!("Window '{}' not found", full_name))?;

        // Use window_id (not tab_id) because kitty's --match id:N resolves
        // window IDs, not tab IDs.
        self.kitten_cmd()
            .args(&["focus-tab", "--match", &format!("id:{}", target.window_id)])
            .run()
            .context("Failed to focus tab")?;
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
        let window_id = match self.current_window_id() {
            Some(id) => id,
            None => return Ok(None),
        };

        let panes = self.list_panes()?;
        let current = panes.iter().find(|p| p.window_id == window_id);

        Ok(current.map(|p| p.tab_title.clone()))
    }

    fn get_all_window_names(&self) -> Result<HashSet<String>> {
        let panes = self.list_panes()?;
        let scoped_panes = self.panes_in_current_scope(&panes);

        // Collect unique tab_titles (our window names)
        let names: HashSet<String> = scoped_panes.iter().map(|p| p.tab_title.clone()).collect();

        Ok(names)
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
        // Kitty doesn't support tab insertion ordering via CLI
        // Return None - new tabs will appear at end
        Ok(None)
    }

    fn find_last_window_with_base_handle(
        &self,
        _prefix: &str,
        _base_handle: &str,
    ) -> Result<Option<String>> {
        // Kitty doesn't support tab insertion ordering via CLI
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

    // === Pane Management ===

    fn select_pane(&self, pane_id: &str) -> Result<()> {
        self.kitten_cmd()
            .args(&["focus-window", "--match", &format!("id:{}", pane_id)])
            .run()
            .context("Failed to focus window")?;
        Ok(())
    }

    fn switch_to_pane(&self, pane_id: &str) -> Result<()> {
        // In kitty, focusing a window also focuses its containing tab
        self.select_pane(pane_id)
    }

    fn respawn_pane(&self, pane_id: &str, cwd: &Path, cmd: Option<&str>) -> Result<String> {
        // Unified approach: split the current pane, then close the original.
        // This preserves tab position regardless of whether there were siblings.
        // The new window will expand to fill the space of the closed one.
        let new_pane_id =
            self.split_pane_internal(pane_id, SplitDirection::Vertical, cwd, None, None, cmd)?;

        // Close old window
        let _ = self
            .kitten_cmd()
            .args(&["close-window", "--match", &format!("id:{}", pane_id)])
            .run();

        Ok(new_pane_id)
    }

    fn capture_pane(&self, pane_id: &str, lines: u16) -> Option<String> {
        let output = self
            .kitten_cmd()
            .args(&["get-text", "--match", &format!("id:{}", pane_id), "--ansi"])
            .run_and_capture_stdout()
            .ok()?;

        // get-text returns all visible content; take last N lines
        let all_lines: Vec<&str> = output.lines().collect();
        let start = all_lines.len().saturating_sub(lines as usize);
        Some(all_lines[start..].join("\n"))
    }

    // === Text I/O ===

    fn send_keys(&self, pane_id: &str, command: &str) -> Result<()> {
        // Send the command text
        self.kitten_cmd()
            .args(&["send-text", "--match", &format!("id:{}", pane_id), command])
            .run()
            .context("Failed to send text to pane")?;

        // Send Enter key
        self.kitten_cmd()
            .args(&["send-text", "--match", &format!("id:{}", pane_id), "\r"])
            .run()
            .context("Failed to send Enter key to pane")?;

        Ok(())
    }

    fn send_keys_to_agent(&self, pane_id: &str, command: &str, agent: Option<&str>) -> Result<()> {
        if agent::resolve_profile(agent).needs_bang_delay() && command.starts_with('!') {
            // Send ! first
            self.kitten_cmd()
                .args(&["send-text", "--match", &format!("id:{}", pane_id), "!"])
                .run()
                .context("Failed to send ! to pane")?;

            // Small delay to let Claude register the !
            thread::sleep(Duration::from_millis(50));

            // Send the rest of the command
            self.kitten_cmd()
                .args(&[
                    "send-text",
                    "--match",
                    &format!("id:{}", pane_id),
                    &command[1..],
                ])
                .run()
                .context("Failed to send keys to pane")?;

            // Send Enter
            self.kitten_cmd()
                .args(&["send-text", "--match", &format!("id:{}", pane_id), "\r"])
                .run()
                .context("Failed to send Enter key to pane")?;

            Ok(())
        } else {
            self.send_keys(pane_id, command)
        }
    }

    fn send_key(&self, pane_id: &str, key: &str) -> Result<()> {
        // Translate tmux key names to ANSI escape sequences for kitty.
        // The dashboard sends tmux-style names like "BSpace", "Enter", etc.
        let translated = match key {
            "BSpace" => "\x7f",
            "Enter" => "\r",
            "Tab" => "\t",
            "Up" => "\x1b[A",
            "Down" => "\x1b[B",
            "Right" => "\x1b[C",
            "Left" => "\x1b[D",
            "Escape" => "\x1b",
            _ => key,
        };
        self.kitten_cmd()
            .args(&[
                "send-text",
                "--match",
                &format!("id:{}", pane_id),
                translated,
            ])
            .run()
            .context("Failed to send key to pane")?;
        Ok(())
    }

    fn paste_multiline(&self, pane_id: &str, content: &str) -> Result<()> {
        // Use bracketed paste mode
        self.kitten_cmd()
            .args(&[
                "send-text",
                "--match",
                &format!("id:{}", pane_id),
                "--bracketed-paste",
                content,
            ])
            .run()
            .context("Failed to paste content to pane")?;

        // Send Enter to submit
        self.kitten_cmd()
            .args(&["send-text", "--match", &format!("id:{}", pane_id), "\r"])
            .run()
            .context("Failed to send Enter after paste")?;

        Ok(())
    }

    // === Shell ===

    fn get_default_shell(&self) -> Result<String> {
        // Kitty doesn't have a config query CLI
        // Use $SHELL or fall back to /bin/bash
        std::env::var("SHELL").or_else(|_| Ok("/bin/bash".to_string()))
    }

    fn create_handshake(&self) -> Result<Box<dyn PaneHandshake>> {
        Ok(Box::new(UnixPipeHandshake::new()?))
    }

    // === Status ===

    fn set_status(&self, pane_id: &str, icon: &str, auto_clear_on_focus: bool) -> Result<()> {
        // Use kitty user variables for status
        // This stores the status per-window, which can be read by custom tab bar scripts
        let match_arg = format!("id:{}", pane_id);
        let _ = self
            .kitten_cmd()
            .args(&[
                "set-user-vars",
                "--match",
                &match_arg,
                &format!("workmux_status={}", icon),
            ])
            .run();

        // Set auto-clear flag so the watcher can clear status on focus
        let auto_clear_val = if auto_clear_on_focus { "1" } else { "" };
        let _ = self
            .kitten_cmd()
            .args(&[
                "set-user-vars",
                "--match",
                &match_arg,
                &format!("workmux_auto_clear={}", auto_clear_val),
            ])
            .run();

        Ok(())
    }

    fn clear_status(&self, pane_id: &str) -> Result<()> {
        // Clear by setting empty value
        let _ = self
            .kitten_cmd()
            .args(&[
                "set-user-vars",
                "--match",
                &format!("id:{}", pane_id),
                "workmux_status=",
            ])
            .run();
        Ok(())
    }

    fn ensure_status_format(&self, _pane_id: &str) -> Result<()> {
        // No-op for kitty - status is displayed via user variables
        // Users need custom tab_bar.py to display status icons
        Ok(())
    }

    // === Multi-Session/Workspace Support ===

    fn current_session(&self) -> Option<String> {
        // Kitty doesn't have named sessions like tmux
        // Use OS window ID as a pseudo-session identifier
        self.current_os_window_id()
            .map(|id| format!("os-window-{}", id))
    }

    fn get_all_window_names_all_sessions(&self) -> Result<HashSet<String>> {
        // Return all tab titles across all OS windows
        let panes = self.list_panes()?;
        let names: HashSet<String> = panes.iter().map(|p| p.tab_title.clone()).collect();
        Ok(names)
    }

    // === State Reconciliation ===

    fn instance_id(&self) -> String {
        // Use KITTY_LISTEN_ON socket path as instance ID
        std::env::var("KITTY_LISTEN_ON").unwrap_or_else(|_| "default".to_string())
    }

    fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>> {
        // Parse pane ID, returning None if it's not a valid number
        let pane_id_num: u64 = match pane_id.parse() {
            Ok(id) => id,
            Err(_) => return Ok(None),
        };

        let panes = self.list_panes()?;
        let pane = panes.into_iter().find(|p| p.window_id == pane_id_num);

        match pane {
            Some(p) => Ok(Some(LivePaneInfo {
                pid: p.foreground_pid.unwrap_or(p.pid),
                current_command: p
                    .foreground_command
                    .unwrap_or_else(|| "unknown".to_string()),
                working_dir: p.cwd,
                title: if p.title.is_empty() {
                    None
                } else {
                    Some(p.title)
                },
                session: Some(format!("os-window-{}", p.os_window_id)),
                window: Some(p.tab_title),
            })),
            None => Ok(None),
        }
    }

    fn get_all_live_pane_info(&self) -> Result<HashMap<String, LivePaneInfo>> {
        let mut result = HashMap::new();

        for p in self.list_panes()? {
            let pane_id = p.window_id.to_string();

            result.insert(
                pane_id,
                LivePaneInfo {
                    pid: p.foreground_pid.unwrap_or(p.pid),
                    current_command: p
                        .foreground_command
                        .unwrap_or_else(|| "unknown".to_string()),
                    working_dir: p.cwd,
                    title: if p.title.is_empty() {
                        None
                    } else {
                        Some(p.title)
                    },
                    session: Some(format!("os-window-{}", p.os_window_id)),
                    window: Some(p.tab_title),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_kitty_backend_name() {
        let backend = KittyBackend::new();
        assert_eq!(backend.name(), "kitty");
    }
}
