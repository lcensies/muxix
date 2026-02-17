//! Zellij multiplexer backend.
//!
//! Limitations:
//! - No percentage-based pane size control (can resize with +/- but not set exact %)
//! - No window insertion order (tabs always append)
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

/// Info about a pane from `zellij action list-panes --json`
#[derive(Debug, serde::Deserialize)]
struct PaneInfo {
    id: u32,
    is_plugin: bool,
    is_focused: bool,
    terminal_command: Option<String>,
    #[serde(default)]
    tab_name: String,
    #[serde(default)]
    title: String,
}

/// Info about a tab from `zellij action list-tabs --json`
#[derive(Debug, serde::Deserialize)]
struct TabInfo {
    position: u32,
    name: String,
    #[allow(dead_code)]
    active: bool,
}

impl TabInfo {
    /// Get tab ID (position is used as tab ID)
    fn tab_id(&self) -> u32 {
        self.position
    }
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
    ///
    /// **Deprecated:** Use `list_tabs()` for richer metadata.
    /// Kept for backward compatibility.
    fn query_tab_names() -> Result<Vec<String>> {
        // Use list_tabs() internally for better efficiency
        let tabs = Self::list_tabs()?;
        Ok(tabs.into_iter().map(|t| t.name).collect())
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

    /// Query all panes using `zellij action list-panes --json`
    fn list_panes() -> Result<Vec<PaneInfo>> {
        let output = Cmd::new("zellij")
            .args(&["action", "list-panes", "--json"])
            .run_and_capture_stdout()
            .context("Failed to list panes")?;

        serde_json::from_str(&output).context("Failed to parse list-panes JSON output")
    }

    /// Query all tabs using `zellij action list-tabs --json`
    fn list_tabs() -> Result<Vec<TabInfo>> {
        let output = Cmd::new("zellij")
            .args(&["action", "list-tabs", "--json"])
            .run_and_capture_stdout()
            .context("Failed to list tabs")?;

        serde_json::from_str(&output).context("Failed to parse list-tabs JSON output")
    }

    /// Get focused pane ID from list-panes output
    fn focused_pane_id() -> Result<u32> {
        let panes = Self::list_panes()?;
        panes
            .iter()
            .find(|p| p.is_focused && !p.is_plugin)
            .map(|p| p.id)
            .ok_or_else(|| anyhow!("No focused terminal pane found"))
    }

    /// Get tab ID by tab name (for future use)
    #[allow(dead_code)]
    fn get_tab_id_by_name(name: &str) -> Result<Option<u32>> {
        let tabs = Self::list_tabs()?;
        Ok(tabs.into_iter().find(|t| t.name == name).map(|t| t.tab_id()))
    }
}

impl Multiplexer for ZellijBackend {
    fn name(&self) -> &'static str {
        "zellij"
    }

    fn capabilities(&self) -> super::MultiplexerCaps {
        super::MultiplexerCaps {
            pane_targeting: false,    // Not reliable with --pane-id, using focus instead
            supports_preview: false,  // Preview requires expensive process spawning
            stable_pane_ids: true,    // Real numeric pane IDs are stable
            exit_on_jump: false,      // Keep dashboard open after jumping
        }
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
        // Fast path: Try environment variable first
        Self::pane_id_from_env()
    }

    fn active_pane_id(&self) -> Option<String> {
        // Reliable path: Query focused pane ID
        Self::focused_pane_id()
            .ok()
            .map(|id| format!("terminal_{}", id))
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

    /// Create a new tab in Zellij.
    /// Returns: Tab name (used as "pane_id" for Zellij operations)
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

        // Explicitly switch to the new tab to ensure focus (avoid race conditions)
        Cmd::new("zellij")
            .args(&["action", "go-to-tab-name", &full_name])
            .run()
            .context("Failed to switch to newly created tab")?;

        // Small delay to ensure tab is fully ready and focused
        std::thread::sleep(std::time::Duration::from_millis(50));

        // Query the focused pane ID (the new tab's initial pane)
        let pane_id = Self::focused_pane_id()
            .with_context(|| format!("Failed to get pane ID for new tab '{}'", full_name))?;

        Ok(format!("terminal_{}", pane_id))
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

        // Use list_tabs() for richer metadata and better efficiency
        let tabs = Self::list_tabs()?;
        Ok(tabs.into_iter().map(|t| t.name).collect())
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

        debug!(
            "switch_to_pane: looking for pane_id '{}', found {} agents",
            pane_id,
            agents.len()
        );

        // Find the agent with matching pane_id
        if let Some(agent) = agents.iter().find(|a| a.pane_id == pane_id) {
            debug!(
                "switch_to_pane: found agent, switching to tab '{}'",
                agent.window_name
            );
            // Switch to the tab using go-to-tab-name
            Cmd::new("zellij")
                .args(&["action", "go-to-tab-name", &agent.window_name])
                .run()
                .with_context(|| format!("Failed to switch to tab '{}'", agent.window_name))?;
            debug!(
                "switch_to_pane: successfully switched to tab '{}'",
                agent.window_name
            );
            Ok(())
        } else {
            warn!(
                "Could not find agent with pane_id '{}' in state store",
                pane_id
            );
            debug!(
                "Available pane_ids: {:?}",
                agents.iter().map(|a| &a.pane_id).collect::<Vec<_>>()
            );
            Err(anyhow!("Pane '{}' not found in state store", pane_id))
        }
    }

    fn respawn_pane(&self, pane_id: &str, cwd: &Path, cmd: Option<&str>) -> Result<String> {
        // Zellij doesn't have respawn-pane; send cd + command to currently focused pane
        // Note: respawn_pane is always called on a newly created/focused pane, so we don't
        // need --pane-id here (commands go to focused pane)
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

        // Return the same pane ID (respawn keeps the same pane)
        Ok(pane_id.to_string())
    }

    fn capture_pane(&self, _pane_id: &str, _lines: u16) -> Option<String> {
        // Zellij limitation: dump-screen always captures the focused pane,
        // not the pane specified by pane_id. When the dashboard is focused,
        // it captures itself, creating a recursive loop. We detect this and
        // return None to prevent the recursion.

        // Use PID + thread ID + timestamp for thread-safe temp file naming
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let thread_id = std::thread::current().id();
        let temp_path = std::env::temp_dir().join(format!(
            "zellij_capture_{}_{:?}_{}",
            std::process::id(),
            thread_id,
            timestamp
        ));
        let temp_str = temp_path.to_string_lossy();

        if Cmd::new("zellij")
            .args(&["action", "dump-screen", &temp_str])
            .run()
            .is_ok()
        {
            if let Ok(content) = std::fs::read_to_string(&temp_path) {
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
        // Note: Zellij's --pane-id flag seems unreliable during setup, so we rely on focus
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
        // Note: This is typically called right after respawn_pane, so the pane is focused
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
                debug!(
                    target_index,
                    current_index, steps, "Navigated backwards to focused pane"
                );
            } else if target_index > current_index {
                // Navigate forwards
                let steps = target_index - current_index;
                for _ in 0..steps {
                    Cmd::new("zellij")
                        .args(&["action", "focus-next-pane"])
                        .run()
                        .context("Failed to navigate to next pane")?;
                }
                debug!(
                    target_index,
                    current_index, steps, "Navigated forwards to focused pane"
                );
            }
        }

        Ok(super::types::PaneSetupResult {
            focus_pane_id: focus_pane_id.unwrap_or_else(|| pane_ids[0].clone()),
        })
    }

    /// Split a pane in Zellij.
    ///
    /// **Zellij CLI Limitations:**
    /// - `target_pane_id` is ignored - Zellij's `new-pane` command doesn't support
    ///   targeting specific panes for splitting (always splits the focused pane).
    /// - `size`/`percentage` are ignored - all splits are 50/50.
    ///
    /// **Returns:** The real pane ID of the newly created pane.
    fn split_pane(
        &self,
        target_pane_id: &str,
        direction: &SplitDirection,
        cwd: &Path,
        _size: Option<u16>,
        _percentage: Option<u8>,
        command: Option<&str>,
    ) -> Result<String> {
        debug!(
            "split_pane: target_pane_id '{}' (note: new-pane splits focused pane only)",
            target_pane_id
        );

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

        // The new pane is now focused, query its real pane ID
        let pane_id = Self::focused_pane_id()
            .context("Failed to get pane ID for new split pane")?;

        Ok(format!("terminal_{}", pane_id))
    }

    // === State Reconciliation ===

    fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>> {
        let panes = Self::list_panes()?;

        // Extract numeric ID from "terminal_X"
        let numeric_id: u32 = pane_id
            .strip_prefix("terminal_")
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| anyhow!("Invalid pane_id: {}", pane_id))?;

        // Find pane by ID
        let pane = match panes.iter().find(|p| p.id == numeric_id && !p.is_plugin) {
            Some(p) => p,
            None => return Ok(None), // Pane doesn't exist
        };

        // Extract command from terminal_command (e.g., "zsh" from "/bin/zsh")
        let current_command = pane
            .terminal_command
            .as_deref()
            .and_then(|cmd| cmd.split_whitespace().next())
            .unwrap_or("")
            .split('/')
            .last()
            .unwrap_or("")
            .to_string();

        Ok(Some(LivePaneInfo {
            pid: 0, // Zellij doesn't expose PID
            current_command,
            working_dir: std::env::current_dir().unwrap_or_default(),
            title: Some(pane.title.clone()).filter(|t| !t.is_empty()),
            session: Self::session_name(),
            window: Some(pane.tab_name.clone()).filter(|t| !t.is_empty()),
        }))
    }

    fn validate_agent_alive(&self, state: &crate::state::AgentState, _cached_tabs: Option<&[String]>) -> Result<bool> {
        use std::time::{Duration, SystemTime};

        // Performance optimization: Check heartbeat first (fast path)
        // If heartbeat is recent, skip expensive pane query
        if let Some(last_heartbeat) = state.last_heartbeat {
            let heartbeat_fresh_threshold = Duration::from_secs(60); // 1 minute
            if let Ok(now) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                let heartbeat_age_secs = now.as_secs().saturating_sub(last_heartbeat);
                if heartbeat_age_secs < heartbeat_fresh_threshold.as_secs() {
                    return Ok(true); // Recent heartbeat - agent is alive
                }
                // If heartbeat is stale (> 5 minutes), agent is likely dead
                if heartbeat_age_secs > 300 {
                    return Ok(false);
                }
            }
        }

        // Primary validation: Check if pane exists
        let pane_info = self.get_live_pane_info(&state.pane_key.pane_id)?;
        let pane_info = match pane_info {
            Some(info) => info,
            None => return Ok(false), // Pane doesn't exist
        };

        // Secondary validation: Check if command matches stored command
        // This detects if the agent process was killed and replaced with something else
        if !state.command.is_empty() && !pane_info.current_command.is_empty() {
            // Extract base command name for comparison
            let expected_base = state.command.split('/').last().unwrap_or(&state.command);
            let actual_base = pane_info.current_command.split('/').last().unwrap_or(&pane_info.current_command);

            if expected_base != actual_base {
                debug!(
                    "Agent validation: command mismatch - expected '{}', got '{}'",
                    expected_base, actual_base
                );
                return Ok(false); // Different command running
            }
        }

        Ok(true) // Agent is valid
    }

    fn get_all_live_pane_info(&self) -> Result<std::collections::HashMap<String, LivePaneInfo>> {
        use std::collections::HashMap;

        let mut result = HashMap::new();

        // Use list-panes to get all panes (not just focused ones)
        let panes = Self::list_panes()?;

        for pane in panes {
            // Skip plugin panes, only include terminal panes
            if pane.is_plugin {
                continue;
            }

            let pane_id = format!("terminal_{}", pane.id);

            // Extract command from terminal_command (e.g., "zsh" from "/bin/zsh")
            let current_command = pane
                .terminal_command
                .as_deref()
                .and_then(|cmd| cmd.split_whitespace().next())
                .unwrap_or("")
                .split('/')
                .last()
                .unwrap_or("")
                .to_string();

            result.insert(
                pane_id,
                LivePaneInfo {
                    pid: 0, // Zellij doesn't expose PID
                    current_command,
                    working_dir: std::env::current_dir().unwrap_or_default(),
                    title: Some(pane.title.clone()).filter(|t| !t.is_empty()),
                    session: Self::session_name(),
                    window: Some(pane.tab_name.clone()).filter(|t| !t.is_empty()),
                },
            );
        }

        Ok(result)
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
