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

    /// Check if content contains dashboard UI patterns
    fn contains_dashboard_ui(content: &str) -> bool {
        // Check for distinctive dashboard UI elements
        // The "Preview:" section is unique to the dashboard
        content.contains("Preview:")
            || (content.contains("[i] input") && content.contains("[d] diff"))
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
        // Zellij doesn't support selecting panes by arbitrary IDs.
        // This is handled in our setup_panes override using focus navigation.
        Ok(())
    }

    fn switch_to_pane(&self, pane_id: &str) -> Result<()> {
        // Zellij can't switch to arbitrary panes, but we can switch to the tab containing the pane.
        // Look up the window name from the state store and switch to that tab.

        use crate::state::StateStore;

        let store = StateStore::new()?;
        let agents = store.load_reconciled_agents(self)?;

        debug!("switch_to_pane: looking for pane_id '{}', found {} agents", pane_id, agents.len());

        // Find the agent with matching pane_id
        if let Some(agent) = agents.iter().find(|a| a.pane_id == pane_id) {
            debug!("switch_to_pane: found agent, switching to tab '{}'", agent.window_name);
            // Switch to the tab using go-to-tab-name
            Cmd::new("zellij")
                .args(&["action", "go-to-tab-name", &agent.window_name])
                .run()
                .with_context(|| format!("Failed to switch to tab '{}'", agent.window_name))?;
            debug!("switch_to_pane: successfully switched to tab '{}'", agent.window_name);
            Ok(())
        } else {
            warn!("Could not find agent with pane_id '{}' in state store", pane_id);
            debug!("Available pane_ids: {:?}", agents.iter().map(|a| &a.pane_id).collect::<Vec<_>>());
            Ok(())
        }
    }

    fn should_exit_on_jump(&self) -> bool {
        // In Zellij, keep the dashboard open after jumping
        // This allows users to easily switch back to the dashboard tab
        false
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
        // Zellij limitation: dump-screen always captures the focused pane,
        // not the pane specified by pane_id. When the dashboard is focused,
        // it captures itself, creating a recursive loop. We detect this and
        // return None to prevent the recursion.

        let temp_path = std::env::temp_dir().join(format!("zellij_capture_{}", std::process::id()));
        let temp_str = temp_path.to_string_lossy();

        if Cmd::new("zellij")
            .args(&["action", "dump-screen", &temp_str])
            .run()
            .is_ok()
        {
            if let Some(content) = std::fs::read_to_string(&temp_path).ok() {
                let _ = std::fs::remove_file(&temp_path);

                // If captured content contains dashboard UI, we're capturing
                // the dashboard itself (not the agent pane). Return None to
                // prevent recursive rendering.
                if Self::contains_dashboard_ui(&content) {
                    return None;
                }

                return Some(content);
            }
            let _ = std::fs::remove_file(&temp_path);
        }

        None
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

    fn setup_panes(
        &self,
        initial_pane_id: &str,
        panes: &[crate::config::PaneConfig],
        working_dir: &Path,
        options: super::types::PaneSetupOptions<'_>,
        config: &crate::config::Config,
        task_agent: Option<&str>,
    ) -> Result<super::types::PaneSetupResult> {
        use super::{agent, util};

        // Zellij-specific implementation with focus navigation support
        if panes.is_empty() {
            return Ok(super::types::PaneSetupResult {
                focus_pane_id: initial_pane_id.to_string(),
            });
        }

        let mut focus_pane_id: Option<String> = None;
        let mut focus_pane_index: Option<usize> = None;
        let mut pane_ids: Vec<String> = vec![initial_pane_id.to_string()];
        let effective_agent = task_agent.or(config.agent.as_deref());
        let shell = self.get_default_shell()?;

        for (i, pane_config) in panes.iter().enumerate() {
            let is_first = i == 0;

            // Skip non-first panes that have no split direction
            if !is_first && pane_config.split.is_none() {
                continue;
            }

            // Resolve command: handle <agent> placeholder and prompt injection
            let adjusted_command = util::resolve_pane_command(
                pane_config.command.as_deref(),
                options.run_commands,
                options.prompt_file_path,
                working_dir,
                effective_agent,
                &shell,
            );

            let pane_id = if let Some(resolved) = adjusted_command {
                // Spawn with handshake so we can send the command after shell is ready
                let handshake = self.create_handshake()?;
                let script = handshake.script_content(&shell);

                let spawned_id = if is_first {
                    self.respawn_pane(&pane_ids[0], working_dir, Some(&script))?
                } else {
                    let direction = pane_config.split.as_ref().unwrap();
                    let target_idx = pane_config.target.unwrap_or(pane_ids.len() - 1);
                    let target = pane_ids
                        .get(target_idx)
                        .ok_or_else(|| anyhow!("Invalid target pane index: {}", target_idx))?;
                    self.split_pane(
                        target,
                        direction,
                        working_dir,
                        pane_config.size,
                        pane_config.percentage,
                        Some(&script),
                    )?
                };

                handshake.wait()?;
                let _ = self.clear_pane(&spawned_id);
                self.send_keys(&spawned_id, &resolved.command)?;

                // Set working status for agent panes with injected prompts
                if resolved.prompt_injected
                    && agent::resolve_profile(effective_agent).needs_auto_status()
                {
                    let icon = config.status_icons.working();
                    if config.status_format.unwrap_or(true) {
                        let _ = self.ensure_status_format(&spawned_id);
                    }
                    let _ = self.set_status(&spawned_id, icon, false);
                }

                spawned_id
            } else if is_first {
                // No command for first pane - keep as-is
                pane_ids[0].clone()
            } else {
                // No command - just split
                let direction = pane_config.split.as_ref().unwrap();
                let target_idx = pane_config.target.unwrap_or(pane_ids.len() - 1);
                let target = pane_ids
                    .get(target_idx)
                    .ok_or_else(|| anyhow!("Invalid target pane index: {}", target_idx))?;
                self.split_pane(
                    target,
                    direction,
                    working_dir,
                    pane_config.size,
                    pane_config.percentage,
                    None,
                )?
            };

            if is_first {
                pane_ids[0] = pane_id.clone();
            } else {
                pane_ids.push(pane_id.clone());
            }

            if pane_config.focus {
                focus_pane_id = Some(pane_id);
                focus_pane_index = Some(i);
            }
        }

        // Zellij-specific: navigate to the focused pane
        // After pane creation, the last created pane has focus
        if let Some(target_index) = focus_pane_index {
            let current_index = pane_ids.len() - 1;
            if target_index < current_index {
                // Navigate backwards
                let steps = current_index - target_index;
                for _ in 0..steps {
                    Cmd::new("zellij")
                        .args(&["action", "focus-previous-pane"])
                        .run()
                        .context("Failed to navigate to previous pane")?;
                }
                debug!(target_index, current_index, steps, "Navigated backwards to focused pane");
            } else if target_index > current_index {
                // Navigate forwards
                let steps = target_index - current_index;
                for _ in 0..steps {
                    Cmd::new("zellij")
                        .args(&["action", "focus-next-pane"])
                        .run()
                        .context("Failed to navigate to next pane")?;
                }
                debug!(target_index, current_index, steps, "Navigated forwards to focused pane");
            }
        }

        Ok(super::types::PaneSetupResult {
            focus_pane_id: focus_pane_id.unwrap_or_else(|| pane_ids[0].clone()),
        })
    }

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
        // For focused panes, return accurate info. For unfocused panes,
        // return fallback data to allow state persistence.
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
                    working_dir: std::env::current_dir().unwrap_or_default(),
                    title: None,
                    session: Self::session_name(),
                    window: Self::focused_tab_name(),
                }));
            }
        }

        // For unfocused panes: return fallback data to allow state persistence.
        // Validation is handled by validate_agent_alive() instead.
        Ok(Some(LivePaneInfo {
            pid: 0,
            current_command: String::new(),
            working_dir: std::env::current_dir().unwrap_or_default(),
            title: None,
            session: Self::session_name(),
            window: None, // Can't determine for unfocused panes
        }))
    }

    fn validate_agent_alive(&self, state: &crate::state::AgentState) -> Result<bool> {
        use std::time::{Duration, SystemTime};

        // For Zellij, we can't validate PID or command for unfocused panes.
        // Instead, we use tab-level validation:
        // 1. Check if the tab (window) still exists
        // 2. Check if the state is stale (no updates in > 1 hour)

        // Check 1: Does the tab still exist?
        if let Some(window_name) = &state.window_name {
            if !self.window_exists_by_full_name(window_name)? {
                return Ok(false); // Tab was closed
            }
        } else {
            // No window name stored - this is an old state file or error.
            // Be conservative and keep it (don't delete valid agents)
            return Ok(true);
        }

        // Check 2: Is the state stale? (no updates in > 1 hour)
        let stale_threshold = Duration::from_secs(3600);
        if let Ok(now) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            let state_age_secs = now.as_secs().saturating_sub(state.updated_ts);
            if state_age_secs > stale_threshold.as_secs() {
                return Ok(false); // Stale agent
            }
        }

        Ok(true) // Agent is valid
    }

    fn get_all_live_pane_info(&self) -> Result<std::collections::HashMap<String, LivePaneInfo>> {
        use std::collections::HashMap;

        let mut result = HashMap::new();

        // Zellij's list-clients only shows info for currently visible clients
        // This is a limitation of zellij's CLI - we can't query all panes
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
                    pid: 0, // Zellij doesn't expose PID
                    current_command,
                    working_dir: std::env::current_dir().unwrap_or_default(),
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
