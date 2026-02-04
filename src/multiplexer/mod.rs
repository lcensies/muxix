//! Multiplexer abstraction layer for terminal multiplexer backends.
//!
//! This module provides a trait-based abstraction that allows workmux to work
//! with different terminal multiplexers (tmux, WezTerm) interchangeably.

pub mod agent;
pub mod handshake;
pub mod kitty;
pub mod tmux;
pub mod types;
pub mod util;
pub mod wezterm;

use anyhow::{Result, anyhow};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub use handshake::PaneHandshake;
pub use tmux::TmuxBackend;
pub use types::*;

use crate::config::{Config, PaneConfig, SplitDirection};

/// Main trait for terminal multiplexer backends.
///
/// Implementations must be Send + Sync to allow sharing via Arc<dyn Multiplexer>.
pub trait Multiplexer: Send + Sync {
    /// Returns the name of this backend (e.g., "tmux", "wezterm")
    fn name(&self) -> &'static str;

    // === Server/Session ===

    /// Check if the multiplexer server is running
    fn is_running(&self) -> Result<bool>;

    /// Get the current pane ID from environment (TMUX_PANE or WEZTERM_PANE)
    fn current_pane_id(&self) -> Option<String>;

    /// Query the active pane ID directly from the multiplexer.
    /// More reliable than current_pane_id() in run-shell contexts (keybindings)
    /// where the env var may be stale or missing.
    fn active_pane_id(&self) -> Option<String>;

    /// Get the working directory of the active pane in the current client's session
    fn get_client_active_pane_path(&self) -> Result<PathBuf>;

    // === Window/Tab Management ===

    /// Create a new window/tab with the given parameters
    fn create_window(&self, params: CreateWindowParams) -> Result<String>;

    /// Kill a window by its full name (including prefix)
    fn kill_window(&self, full_name: &str) -> Result<()>;

    /// Schedule a window to close after a delay
    fn schedule_window_close(&self, full_name: &str, delay: Duration) -> Result<()>;

    /// Run a deferred script in the background (for cleanup operations).
    /// For tmux, this uses `run-shell`. For other backends, may use different mechanisms.
    fn run_deferred_script(&self, script: &str) -> Result<()>;

    /// Generate a shell command string to select/focus a window by full name.
    /// Used in deferred scripts that run asynchronously via `run_deferred_script`.
    fn shell_select_window_cmd(&self, full_name: &str) -> Result<String>;

    /// Generate a shell command string to close/kill a window by full name.
    /// Used in deferred scripts that run asynchronously via `run_deferred_script`.
    fn shell_kill_window_cmd(&self, full_name: &str) -> Result<String>;

    /// Select (focus) a window by prefix and name
    fn select_window(&self, prefix: &str, name: &str) -> Result<()>;

    /// Check if a window exists by prefix and name
    fn window_exists(&self, prefix: &str, name: &str) -> Result<bool>;

    /// Check if a window exists by its full name
    fn window_exists_by_full_name(&self, full_name: &str) -> Result<bool>;

    /// Get the current window name, if running inside the multiplexer
    fn current_window_name(&self) -> Result<Option<String>>;

    /// Get all window names in the current session
    fn get_all_window_names(&self) -> Result<HashSet<String>>;

    /// Filter a list of window names, returning only those that still exist
    fn filter_active_windows(&self, windows: &[String]) -> Result<Vec<String>>;

    /// Find the last window (by index) that starts with the given prefix
    fn find_last_window_with_prefix(&self, prefix: &str) -> Result<Option<String>>;

    /// Find the last window that belongs to a specific base handle group
    fn find_last_window_with_base_handle(
        &self,
        prefix: &str,
        base_handle: &str,
    ) -> Result<Option<String>>;

    /// Wait until all specified windows are closed
    fn wait_until_windows_closed(&self, full_window_names: &[String]) -> Result<()>;

    // === Pane Management ===

    /// Select (focus) a pane by ID
    fn select_pane(&self, pane_id: &str) -> Result<()>;

    /// Switch to a pane (may also switch windows/tabs as needed)
    fn switch_to_pane(&self, pane_id: &str) -> Result<()>;

    /// Respawn a pane with optional command. Returns the (possibly new) pane ID.
    fn respawn_pane(&self, pane_id: &str, cwd: &Path, cmd: Option<&str>) -> Result<String>;

    /// Capture the content of a pane
    fn capture_pane(&self, pane_id: &str, lines: u16) -> Option<String>;

    // === Text I/O ===

    /// Send keys (command + Enter) to a pane
    fn send_keys(&self, pane_id: &str, command: &str) -> Result<()>;

    /// Send keys to an agent pane, with special handling for Claude's ! prefix
    fn send_keys_to_agent(&self, pane_id: &str, command: &str, agent: Option<&str>) -> Result<()>;

    /// Send a single key to a pane
    fn send_key(&self, pane_id: &str, key: &str) -> Result<()>;

    /// Paste multiline content to a pane (using bracketed paste)
    fn paste_multiline(&self, pane_id: &str, content: &str) -> Result<()>;

    // === Shell ===

    /// Get the default shell for new panes
    fn get_default_shell(&self) -> Result<String>;

    /// Create a handshake mechanism for synchronizing shell startup
    fn create_handshake(&self) -> Result<Box<dyn PaneHandshake>>;

    // === Status ===

    /// Set status icon for a pane.
    ///
    /// If `auto_clear_on_focus` is true, the status will be automatically cleared
    /// when the window receives focus (used for "waiting" and "done" statuses).
    fn set_status(&self, pane_id: &str, icon: &str, auto_clear_on_focus: bool) -> Result<()>;

    /// Clear status from a pane
    fn clear_status(&self, pane_id: &str) -> Result<()>;

    /// Ensure the status format is configured (for backends that need it)
    fn ensure_status_format(&self, pane_id: &str) -> Result<()>;

    // === Pane Setup ===

    /// Split a pane, returning the new pane ID.
    fn split_pane(
        &self,
        target_pane_id: &str,
        direction: &SplitDirection,
        cwd: &Path,
        size: Option<u16>,
        percentage: Option<u8>,
        command: Option<&str>,
    ) -> Result<String>;

    /// Setup panes in a window according to configuration.
    ///
    /// Default implementation handles the full orchestration: command resolution,
    /// handshake-based shell synchronization, command injection, and auto-status.
    /// Backends only need to implement `respawn_pane`, `split_pane`, and other
    /// primitive trait methods.
    fn setup_panes(
        &self,
        initial_pane_id: &str,
        panes: &[PaneConfig],
        working_dir: &Path,
        options: PaneSetupOptions<'_>,
        config: &Config,
        task_agent: Option<&str>,
    ) -> Result<PaneSetupResult> {
        if panes.is_empty() {
            return Ok(PaneSetupResult {
                focus_pane_id: initial_pane_id.to_string(),
            });
        }

        let mut focus_pane_id: Option<String> = None;
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

                // Detect if this is an agent pane for sandbox targeting
                let is_agent_pane = pane_config.command.as_deref().is_some_and(|cmd| {
                    cmd == "<agent>"
                        || effective_agent.is_some_and(|a| crate::config::is_agent_command(cmd, a))
                });

                // Apply sandbox wrapping if enabled for this pane type
                let final_command = if config.sandbox.is_enabled() {
                    let should_wrap = match config.sandbox.target() {
                        crate::config::SandboxTarget::All => true,
                        crate::config::SandboxTarget::Agent => is_agent_pane,
                    };
                    if should_wrap {
                        // Use worktree_root for mounting, working_dir for cwd
                        let wt_root = options.worktree_root.unwrap_or(working_dir);

                        // Inject skip-permissions flag for agents that support it
                        // (sandbox provides the security boundary, so permission
                        // prompts are unnecessary and break autonomous workflow)
                        let command_to_wrap = {
                            let profile =
                                crate::multiplexer::agent::resolve_profile(effective_agent);
                            if let Some(flag) = profile.skip_permissions_flag() {
                                util::inject_skip_permissions_flag(&resolved.command, flag)
                            } else {
                                resolved.command.clone()
                            }
                        };

                        // Choose backend based on config
                        let wrap_result = match config.sandbox.backend() {
                            crate::config::SandboxBackend::Container => {
                                crate::sandbox::wrap_for_container(
                                    &command_to_wrap,
                                    &config.sandbox,
                                    wt_root,
                                    working_dir,
                                )
                            }
                            crate::config::SandboxBackend::Lima => {
                                let vm_name = options.lima_vm_name.ok_or_else(|| {
                                    anyhow!(
                                        "Lima VM name missing despite sandbox wrap request. \
                                         This is a bug in workmux."
                                    )
                                })?;
                                crate::sandbox::wrap_for_lima(
                                    &command_to_wrap,
                                    config,
                                    vm_name,
                                    working_dir,
                                )
                            }
                        };

                        // Fail closed: if sandbox is enabled but wrapping fails, don't fall back to unsandboxed
                        match wrap_result {
                            Ok(wrapped) => wrapped,
                            Err(e) => {
                                return Err(anyhow!(
                                    "Sandbox is enabled but failed to wrap command: {}. \
                                     To disable sandbox, set 'sandbox.enabled: false' in config.",
                                    e
                                ));
                            }
                        }
                    } else {
                        resolved.command.clone()
                    }
                } else {
                    resolved.command.clone()
                };

                self.send_keys(&spawned_id, &final_command)?;

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
            }
        }

        Ok(PaneSetupResult {
            focus_pane_id: focus_pane_id.unwrap_or_else(|| pane_ids[0].clone()),
        })
    }

    // === Multi-Session/Workspace Support ===

    /// Get the current session/workspace name, if determinable.
    ///
    /// Returns None if not running inside the multiplexer.
    /// For tmux, this is the session name. For WezTerm, this is the workspace name.
    #[allow(dead_code)] // Reserved for future multi-session features
    fn current_session(&self) -> Option<String> {
        None // Default: can't determine
    }

    /// Get all window names across ALL sessions/workspaces.
    ///
    /// Default implementation returns same as get_all_window_names() (single session).
    /// WezTerm overrides this to return windows from all workspaces.
    #[allow(dead_code)] // Reserved for future multi-session features
    fn get_all_window_names_all_sessions(&self) -> Result<HashSet<String>> {
        self.get_all_window_names()
    }

    // === State Reconciliation ===

    /// Get the backend instance identifier (socket path, mux ID, etc.).
    ///
    /// This is used to create unique state file paths when multiple instances
    /// of the same backend are running (e.g., multiple tmux servers).
    ///
    /// For tmux: socket path or "default" for standard socket
    /// For WezTerm: mux domain ID or workspace name
    fn instance_id(&self) -> String;

    /// Get live pane info including PID and current command.
    ///
    /// Returns None if pane does not exist. Used during state reconciliation
    /// to validate stored state against actual pane state.
    fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>>;

    /// Get live pane info for all panes at once (batched query).
    ///
    /// Returns a HashMap from pane_id to LivePaneInfo. This is more efficient
    /// than calling get_live_pane_info repeatedly when validating many panes.
    fn get_all_live_pane_info(&self) -> Result<std::collections::HashMap<String, LivePaneInfo>>;
}

/// Detect which backend to use based on environment.
///
/// Auto-detects from multiplexer environment variables:
/// - `$WEZTERM_PANE` set → WezTerm
/// - `$KITTY_WINDOW_ID` set → Kitty
/// - `$TMUX` set → tmux
/// - None → defaults to tmux (for backward compatibility)
pub fn detect_backend() -> BackendType {
    // Auto-detect from environment
    if std::env::var("WEZTERM_PANE").is_ok() {
        return BackendType::WezTerm;
    }

    if std::env::var("KITTY_WINDOW_ID").is_ok() {
        return BackendType::Kitty;
    }

    if std::env::var("TMUX").is_ok() {
        return BackendType::Tmux;
    }

    // Default to tmux for backward compatibility
    BackendType::Tmux
}

/// Create a backend instance based on the backend type.
pub fn create_backend(backend_type: BackendType) -> Arc<dyn Multiplexer> {
    match backend_type {
        BackendType::Tmux => Arc::new(TmuxBackend::new()),
        BackendType::WezTerm => Arc::new(wezterm::WezTermBackend::new()),
        BackendType::Kitty => Arc::new(kitty::KittyBackend::new()),
    }
}
