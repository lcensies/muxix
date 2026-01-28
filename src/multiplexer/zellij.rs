//! Zellij multiplexer backend.
//!
//! Limitations:
//! - No pane targeting (commands go to focused pane, not specific pane ID)
//! - No percentage-based pane size control (can resize with +/- but not set exact %)
//! - No window insertion order (tabs always append)
//! - One status per tab (state tracked by tab name, not pane ID)
//! - No visual status indicator (set_status is a no-op)

use anyhow::{Context, Result, anyhow};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tracing::{debug, warn};

use crate::cmd::Cmd;
use crate::config::SplitDirection;

use super::handshake::UnixPipeHandshake;
use super::types::{CreateWindowParams, LivePaneInfo};
use super::{Multiplexer, PaneHandshake};

/// Zellij multiplexer backend.
pub struct ZellijBackend {
    _private: (),
}

/// Info about a client/pane from `zellij action list-clients`
#[derive(Debug)]
struct ClientInfo {
    pane_id: String,         // e.g., "terminal_1", "plugin_2"
    running_command: String, // e.g., "vim /tmp/file.txt", "zsh"
}

impl Default for ZellijBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl ZellijBackend {
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Check if inside a zellij session
    fn is_inside_session() -> bool {
        std::env::var("ZELLIJ").is_ok()
    }

    /// Get session name from environment
    fn session_name() -> Option<String> {
        std::env::var("ZELLIJ_SESSION_NAME").ok()
    }

    /// Get current pane ID from environment (format: terminal_1, plugin_2, etc.)
    fn pane_id_from_env() -> Option<String> {
        std::env::var("ZELLIJ_PANE_ID")
            .ok()
            .map(|id| format!("terminal_{}", id))
    }

    /// Query tab names from zellij
    fn query_tab_names() -> Result<Vec<String>> {
        let output = Cmd::new("zellij")
            .args(&["action", "query-tab-names"])
            .run_and_capture_stdout()?;

        Ok(output.lines().map(|s| s.trim().to_string()).collect())
    }

    /// Get the name of the currently focused tab by parsing dump-layout output.
    fn focused_tab_name() -> Option<String> {
        let output = Cmd::new("zellij")
            .args(&["action", "dump-layout"])
            .run_and_capture_stdout()
            .ok()?;

        // Parse: tab name="TabName" focus=true
        // The focused tab has focus=true in its attributes
        for line in output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("tab ") && trimmed.contains("focus=true") {
                // Extract name="..." from the line
                if let Some(name_start) = trimmed.find("name=\"") {
                    let after_name = &trimmed[name_start + 6..];
                    if let Some(name_end) = after_name.find('"') {
                        return Some(after_name[..name_end].to_string());
                    }
                }
            }
        }

        None
    }

    /// Parse `zellij action list-clients` output
    /// Format: "CLIENT_ID ZELLIJ_PANE_ID RUNNING_COMMAND\n1 terminal_3 vim file.txt"
    fn list_clients() -> Result<Vec<ClientInfo>> {
        let output = Cmd::new("zellij")
            .args(&["action", "list-clients"])
            .run_and_capture_stdout()?;

        let mut clients = Vec::new();
        for line in output.lines().skip(1) {
            // skip header
            // Use split_whitespace to handle variable spacing in output
            let mut parts = line.split_whitespace();
            let _client_id = parts.next(); // skip client ID
            if let Some(pane_id) = parts.next() {
                let running_command: String = parts.collect::<Vec<_>>().join(" ");
                clients.push(ClientInfo {
                    pane_id: pane_id.to_string(),
                    running_command,
                });
            }
        }
        Ok(clients)
    }
}

impl Multiplexer for ZellijBackend {
    fn name(&self) -> &'static str {
        "zellij"
    }

    // === Server/Session ===

    fn is_running(&self) -> Result<bool> {
        if Self::is_inside_session() {
            return Ok(true);
        }
        // Try a simple command to check if zellij is accessible
        Cmd::new("zellij")
            .args(&["action", "dump-screen", "/dev/null"])
            .run_as_check()
    }

    fn current_pane_id(&self) -> Option<String> {
        // ZELLIJ_PANE_ID contains the numeric ID, we prefix with "terminal_"
        Self::pane_id_from_env()
    }

    fn active_pane_id(&self) -> Option<String> {
        // In zellij, we can also try to get this from list-clients
        // but the env var is more reliable in most contexts
        self.current_pane_id()
    }

    fn get_client_active_pane_path(&self) -> Result<PathBuf> {
        // Zellij doesn't expose this via CLI
        // Fall back to current directory
        std::env::current_dir().context("Failed to get current directory")
    }

    fn instance_id(&self) -> String {
        Self::session_name().unwrap_or_else(|| "default".to_string())
    }

    // === Window/Tab Management ===

    fn create_window(&self, params: CreateWindowParams) -> Result<String> {
        let full_name = format!("{}{}", params.prefix, params.name);
        let cwd_str = params
            .cwd
            .to_str()
            .ok_or_else(|| anyhow!("Path contains non-UTF8 characters"))?;

        if params.after_window.is_some() {
            debug!("Zellij does not support window insertion order - ignoring after_window");
        }

        Cmd::new("zellij")
            .args(&[
                "action", "new-tab", "--layout", "default", "--name", &full_name, "--cwd", cwd_str,
            ])
            .run()
            .with_context(|| format!("Failed to create zellij tab '{}'", full_name))?;

        // Stay on the new tab - pane setup expects focus on the new window

        Ok(full_name)
    }

    fn kill_window(&self, full_name: &str) -> Result<()> {
        // Must switch to tab first, then close it
        Cmd::new("zellij")
            .args(&["action", "go-to-tab-name", full_name])
            .run()
            .context("Failed to switch to tab for closing")?;

        Cmd::new("zellij")
            .args(&["action", "close-tab"])
            .run()
            .context("Failed to close zellij tab")?;
        Ok(())
    }

    fn schedule_window_close(&self, full_name: &str, delay: Duration) -> Result<()> {
        // Zellij doesn't have run-shell, spawn a background process
        let delay_secs = delay.as_secs();
        let cmd = format!(
            "sleep {} && zellij action go-to-tab-name '{}' && zellij action close-tab",
            delay_secs,
            full_name.replace('\'', "'\\''")
        );

        std::process::Command::new("sh")
            .args(["-c", &cmd])
            .spawn()
            .context("Failed to spawn delayed close")?;

        Ok(())
    }

    fn select_window(&self, prefix: &str, name: &str) -> Result<()> {
        let full_name = format!("{}{}", prefix, name);
        Cmd::new("zellij")
            .args(&["action", "go-to-tab-name", &full_name])
            .run()
            .context("Failed to select zellij tab")?;
        Ok(())
    }

    fn window_exists(&self, prefix: &str, name: &str) -> Result<bool> {
        let full_name = format!("{}{}", prefix, name);
        self.window_exists_by_full_name(&full_name)
    }

    fn window_exists_by_full_name(&self, full_name: &str) -> Result<bool> {
        if !Self::is_inside_session() {
            return Ok(false);
        }

        let tabs = Self::query_tab_names()?;
        Ok(tabs.iter().any(|t| t == full_name))
    }

    fn current_window_name(&self) -> Result<Option<String>> {
        Ok(Self::focused_tab_name())
    }

    fn get_all_window_names(&self) -> Result<HashSet<String>> {
        if !Self::is_inside_session() {
            return Ok(HashSet::new());
        }

        let tabs = Self::query_tab_names()?;
        Ok(tabs.into_iter().collect())
    }

    fn filter_active_windows(&self, windows: &[String]) -> Result<Vec<String>> {
        let active = self.get_all_window_names()?;
        Ok(windows
            .iter()
            .filter(|w| active.contains(*w))
            .cloned()
            .collect())
    }

    fn find_last_window_with_prefix(&self, _prefix: &str) -> Result<Option<String>> {
        // Zellij doesn't support window ordering
        Ok(None)
    }

    fn find_last_window_with_base_handle(
        &self,
        _prefix: &str,
        _base_handle: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    fn wait_until_windows_closed(&self, full_window_names: &[String]) -> Result<()> {
        use std::thread;

        loop {
            let active = self.get_all_window_names()?;
            if full_window_names.iter().all(|w| !active.contains(w)) {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    // === Pane Management ===

    fn select_pane(&self, _pane_id: &str) -> Result<()> {
        warn!("Zellij does not support selecting panes by ID");
        Ok(())
    }

    fn switch_to_pane(&self, _pane_id: &str) -> Result<()> {
        warn!("Zellij does not support switching to panes by ID");
        Ok(())
    }

    fn respawn_pane(&self, _pane_id: &str, cwd: &Path, cmd: Option<&str>) -> Result<String> {
        // Zellij doesn't have respawn-pane; send cd + command to current pane
        let cwd_str = cwd
            .to_str()
            .ok_or_else(|| anyhow!("Path contains non-UTF8 characters"))?;

        // Send cd command
        let cd_cmd = format!("cd '{}'", cwd_str.replace('\'', "'\\''"));
        Cmd::new("zellij")
            .args(&["action", "write-chars", &cd_cmd])
            .run()?;
        Cmd::new("zellij")
            .args(&["action", "write", "13"]) // Enter
            .run()?;

        // Send actual command if provided
        if let Some(command) = cmd {
            Cmd::new("zellij")
                .args(&["action", "write-chars", command])
                .run()?;
            Cmd::new("zellij").args(&["action", "write", "13"]).run()?;
        }

        // Return current pane ID (respawn keeps the same pane)
        Ok(Self::pane_id_from_env().unwrap_or_else(|| "terminal_0".to_string()))
    }

    fn capture_pane(&self, _pane_id: &str, _lines: u16) -> Option<String> {
        // dump-screen captures entire screen, not specific pane
        // Create a temp file for output
        let temp_path = std::env::temp_dir().join(format!("zellij_capture_{}", std::process::id()));
        let temp_str = temp_path.to_string_lossy();

        if Cmd::new("zellij")
            .args(&["action", "dump-screen", &temp_str])
            .run()
            .is_ok()
        {
            let content = std::fs::read_to_string(&temp_path).ok();
            let _ = std::fs::remove_file(&temp_path);
            content
        } else {
            None
        }
    }

    // === Text I/O ===

    fn send_keys(&self, _pane_id: &str, command: &str) -> Result<()> {
        // write-chars sends to currently focused pane
        Cmd::new("zellij")
            .args(&["action", "write-chars", command])
            .run()
            .context("Failed to send keys")?;

        // Send Enter (ASCII 13)
        Cmd::new("zellij")
            .args(&["action", "write", "13"])
            .run()
            .context("Failed to send Enter")?;
        Ok(())
    }

    fn send_keys_to_agent(&self, pane_id: &str, command: &str, agent: Option<&str>) -> Result<()> {
        use super::agent;

        let profile = agent::resolve_profile(agent);

        if profile.needs_bang_delay() && command.starts_with('!') {
            // Send ! first, wait, then rest of command
            Cmd::new("zellij")
                .args(&["action", "write-chars", "!"])
                .run()?;

            std::thread::sleep(std::time::Duration::from_millis(50));

            Cmd::new("zellij")
                .args(&["action", "write-chars", &command[1..]])
                .run()?;

            Cmd::new("zellij").args(&["action", "write", "13"]).run()?;

            Ok(())
        } else {
            self.send_keys(pane_id, command)
        }
    }

    fn send_key(&self, _pane_id: &str, key: &str) -> Result<()> {
        // Map common key names to ASCII codes
        let code = match key {
            "Enter" => "13",
            "Escape" => "27",
            "Tab" => "9",
            _ => {
                // For single chars, use write-chars
                Cmd::new("zellij")
                    .args(&["action", "write-chars", key])
                    .run()
                    .context("Failed to send key")?;
                return Ok(());
            }
        };

        Cmd::new("zellij")
            .args(&["action", "write", code])
            .run()
            .context("Failed to send key")?;
        Ok(())
    }

    fn paste_multiline(&self, _pane_id: &str, content: &str) -> Result<()> {
        // Send line by line
        for line in content.lines() {
            Cmd::new("zellij")
                .args(&["action", "write-chars", line])
                .run()?;
            Cmd::new("zellij").args(&["action", "write", "13"]).run()?;
        }
        Ok(())
    }

    fn clear_pane(&self, _pane_id: &str) -> Result<()> {
        // Clear the focused pane to hide handshake setup commands
        Cmd::new("zellij")
            .args(&["action", "clear"])
            .run()
            .context("Failed to clear pane")?;
        Ok(())
    }

    // === Shell ===

    fn get_default_shell(&self) -> Result<String> {
        std::env::var("SHELL").or_else(|_| Ok("/bin/sh".to_string()))
    }

    fn create_handshake(&self) -> Result<Box<dyn PaneHandshake>> {
        // Reuse the same Unix pipe handshake as WezTerm
        Ok(Box::new(UnixPipeHandshake::new()?))
    }

    // === Status ===

    fn set_status(&self, _pane_id: &str, _icon: &str, _auto_clear_on_focus: bool) -> Result<()> {
        // No-op: can't target specific panes, and rename-pane would hijack
        // the user's focused pane. Status is tracked in StateStore by tab name.
        Ok(())
    }

    fn clear_status(&self, _pane_id: &str) -> Result<()> {
        // No-op: status is managed by StateStore
        Ok(())
    }

    fn ensure_status_format(&self, _pane_id: &str) -> Result<()> {
        // No-op for zellij
        Ok(())
    }

    // === Pane Setup ===

    fn split_pane(
        &self,
        _target_pane_id: &str,
        direction: &SplitDirection,
        cwd: &Path,
        _size: Option<u16>,
        _percentage: Option<u8>,
        command: Option<&str>,
    ) -> Result<String> {
        // Note: size/percentage ignored - zellij doesn't support percentage-based
        // sizing via CLI. All splits are 50/50.

        let dir_arg = match direction {
            SplitDirection::Horizontal => "right", // panes side-by-side (left/right)
            SplitDirection::Vertical => "down",    // panes stacked (top/bottom)
        };

        let cwd_str = cwd
            .to_str()
            .ok_or_else(|| anyhow!("Path contains non-UTF8 characters"))?;

        Cmd::new("zellij")
            .args(&[
                "action",
                "new-pane",
                "--direction",
                dir_arg,
                "--cwd",
                cwd_str,
            ])
            .run()
            .context("Failed to split pane")?;

        // zellij's --cwd doesn't always work, so send cd command as fallback
        let cd_cmd = format!("cd '{}'", cwd_str.replace('\'', "'\\''"));
        Cmd::new("zellij")
            .args(&["action", "write-chars", &cd_cmd])
            .run()?;
        Cmd::new("zellij").args(&["action", "write", "13"]).run()?;

        // Send command if provided
        if let Some(cmd) = command {
            Cmd::new("zellij")
                .args(&["action", "write-chars", cmd])
                .run()?;
            Cmd::new("zellij").args(&["action", "write", "13"]).run()?;
        }

        // The new pane is now focused, get its ID from env
        // Note: This requires the shell to have set ZELLIJ_PANE_ID
        // For now, return a placeholder - the actual ID will be available
        // once the shell initializes
        Ok(Self::pane_id_from_env().unwrap_or_else(|| format!("terminal_{}", std::process::id())))
    }

    // === State Reconciliation ===

    fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>> {
        // list-clients only shows the focused pane, not arbitrary pane IDs.
        // For zellij, state reconciliation uses query-tab-names instead.
        // This is implemented for interface compliance but returns None
        // unless the requested pane happens to be focused.
        let clients = Self::list_clients()?;

        for client in clients {
            if client.pane_id == pane_id {
                let current_command = client
                    .running_command
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_string();

                return Ok(Some(LivePaneInfo {
                    pid: 0, // Zellij doesn't expose PID
                    current_command,
                    working_dir: PathBuf::new(), // Not available
                    title: None,
                    session: Self::session_name(),
                    window: Self::focused_tab_name(),
                }));
            }
        }

        Ok(None)
    }

    fn get_all_live_pane_info(&self) -> Result<std::collections::HashMap<String, LivePaneInfo>> {
        let mut result = std::collections::HashMap::new();
        let clients = Self::list_clients()?;

        for client in clients {
            let current_command = client
                .running_command
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();

            result.insert(
                client.pane_id,
                LivePaneInfo {
                    pid: 0,
                    current_command,
                    working_dir: PathBuf::new(),
                    title: None,
                    session: Self::session_name(),
                    window: Self::focused_tab_name(),
                },
            );
        }

        Ok(result)
    }

    fn run_deferred_script(&self, script: &str) -> Result<()> {
        fn shell_escape(s: &str) -> String {
            format!("'{}'", s.replace('\'', r#"'\''"#))
        }

        let full_cmd = format!(
            "nohup sh -c {} </dev/null >/dev/null 2>&1 &",
            shell_escape(script)
        );

        std::process::Command::new("sh")
            .args(["-c", &full_cmd])
            .current_dir("/")
            .env_remove("ZELLIJ_PANE_ID")
            .spawn()
            .context("Failed to spawn deferred script")?;

        Ok(())
    }

    fn shell_select_window_cmd(&self, full_name: &str) -> Result<String> {
        fn shell_escape(s: &str) -> String {
            format!("'{}'", s.replace('\'', r#"'\''"#))
        }

        Ok(format!(
            "zellij action go-to-tab-name {} >/dev/null 2>&1",
            shell_escape(full_name)
        ))
    }

    fn shell_kill_window_cmd(&self, full_name: &str) -> Result<String> {
        fn shell_escape(s: &str) -> String {
            format!("'{}'", s.replace('\'', r#"'\''"#))
        }

        // Zellij requires focusing a tab before closing it
        Ok(format!(
            "zellij action go-to-tab-name {} >/dev/null 2>&1; zellij action close-tab >/dev/null 2>&1",
            shell_escape(full_name)
        ))
    }

    fn schedule_cleanup_and_close(
        &self,
        source_window: &str,
        target_window: Option<&str>,
        cleanup_script: &str,
        delay: Duration,
    ) -> Result<()> {
        // Shell-escape helper
        fn shell_escape(s: &str) -> String {
            format!("'{}'", s.replace('\'', r#"'\''"#))
        }

        let delay_secs = delay.as_secs_f64();

        // Build a robust shell script that survives the window closing
        // trap '' HUP ensures the script continues even when the PTY is destroyed
        let mut script = format!("trap '' HUP; sleep {:.1};", delay_secs);

        // 1. Navigate to target (if exists)
        if let Some(target) = target_window {
            script.push_str(&format!(
                " zellij action go-to-tab-name {} >/dev/null 2>&1;",
                shell_escape(target)
            ));
        }

        // 2. Close source tab
        // In Zellij, we must focus the tab to close it
        script.push_str(&format!(
            " zellij action go-to-tab-name {} >/dev/null 2>&1;",
            shell_escape(source_window)
        ));
        script.push_str(" zellij action close-tab >/dev/null 2>&1;");

        // 3. Run cleanup script
        if !cleanup_script.is_empty() {
            script.push(' ');
            script.push_str(cleanup_script);
        }

        debug!(script = script, "zellij:scheduling cleanup and close");

        // Spawn as fully detached background process using nohup
        // - Redirect stdin from /dev/null for proper detachment
        // - Run from root dir to avoid holding a lock on the directory being deleted
        // - Remove ZELLIJ_PANE_ID to avoid confusing zellij CLI
        let full_cmd = format!(
            "nohup sh -c {} </dev/null >/dev/null 2>&1 &",
            shell_escape(&script)
        );

        std::process::Command::new("sh")
            .args(["-c", &full_cmd])
            .current_dir("/")
            .env_remove("ZELLIJ_PANE_ID")
            .spawn()
            .context("Failed to spawn cleanup process")?;

        Ok(())
    }
}
