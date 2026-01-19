//! tmux backend implementation for the Multiplexer trait.
//!
//! This module provides TmuxBackend, which wraps all tmux-specific operations
//! and exposes them through the Multiplexer trait interface.

use anyhow::{Context, Result, anyhow};
use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cmd::Cmd;
use crate::config::{Config, PaneConfig, SplitDirection as ConfigSplitDirection};

use super::handshake::TmuxHandshake;
use super::types::*;
use super::util;
use super::{Multiplexer, PaneHandshake};

/// tmux backend implementation.
///
/// This struct wraps all tmux-specific operations and implements the Multiplexer
/// trait to provide a unified interface with other backends.
#[derive(Debug, Default)]
pub struct TmuxBackend;

impl TmuxBackend {
    /// Create a new TmuxBackend instance.
    pub fn new() -> Self {
        Self
    }

    /// Get the default shell configured in tmux.
    fn get_default_shell_internal(&self) -> Result<String> {
        let output = Cmd::new("tmux")
            .args(&["show-option", "-gqv", "default-shell"])
            .run_and_capture_stdout()?;
        let shell = output.trim();
        if shell.is_empty() {
            Ok("/bin/bash".to_string())
        } else {
            Ok(shell.to_string())
        }
    }

    /// Execute a shell script via tmux run-shell.
    fn run_shell(&self, script: &str) -> Result<()> {
        Cmd::new("tmux")
            .args(&["run-shell", script])
            .run()
            .context("Failed to run shell command via tmux")?;
        Ok(())
    }

    /// Clear the window status display (status bar icon).
    fn clear_window_status_internal(&self, pane_id: &str) {
        let _ = Cmd::new("tmux")
            .args(&["set-option", "-uw", "-t", pane_id, "@workmux_status"])
            .run();
        let _ = Cmd::new("tmux")
            .args(&["set-option", "-uw", "-t", pane_id, "@workmux_status_ts"])
            .run();
    }

    /// Sets the "working" status on a pane.
    fn set_pane_working_status(&self, pane_id: &str, config: &Config) -> Result<()> {
        let icon = config.status_icons.working();

        // Ensure the status format is applied so the icon shows up
        if config.status_format.unwrap_or(true) {
            let _ = self.ensure_status_format(pane_id);
        }

        self.set_status(pane_id, icon, false)?;
        Ok(())
    }

    /// Updates a single tmux format option for the target window to include workmux status.
    fn update_format_option(&self, pane: &str, option: &str) -> Result<()> {
        // Read current format. Try window-level first, fall back to global.
        let window_format = Cmd::new("tmux")
            .args(&["show-option", "-wv", "-t", pane, option])
            .run_and_capture_stdout()
            .ok()
            .filter(|s| !s.is_empty());

        let current = match window_format {
            Some(fmt) => fmt,
            None => Cmd::new("tmux")
                .args(&["show-option", "-gv", option])
                .run_and_capture_stdout()
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "#I:#W#{?window_flags,#{window_flags}, }".to_string()),
        };

        if !current.contains("@workmux_status") {
            let new_format = inject_status_format(&current);
            // Set per-window to avoid affecting other windows/sessions
            Cmd::new("tmux")
                .args(&["set-option", "-w", "-t", pane, option, &new_format])
                .run()?;
        }
        Ok(())
    }

    /// Internal split pane implementation.
    fn split_pane_internal(
        &self,
        target_pane_id: &str,
        direction: &ConfigSplitDirection,
        working_dir: &Path,
        size: Option<u16>,
        percentage: Option<u8>,
        shell_command: Option<&str>,
    ) -> Result<String> {
        let split_arg = match direction {
            ConfigSplitDirection::Horizontal => "-h",
            ConfigSplitDirection::Vertical => "-v",
        };

        let working_dir_str = working_dir
            .to_str()
            .ok_or_else(|| anyhow!("Working directory path contains non-UTF8 characters"))?;

        let mut cmd = Cmd::new("tmux").args(&[
            "split-window",
            split_arg,
            "-t",
            target_pane_id,
            "-c",
            working_dir_str,
            "-P",
            "-F",
            "#{pane_id}",
        ]);

        let size_arg;
        if let Some(p) = percentage {
            size_arg = format!("{}%", p);
            cmd = cmd.args(&["-l", &size_arg]);
        } else if let Some(s) = size {
            size_arg = s.to_string();
            cmd = cmd.args(&["-l", &size_arg]);
        }

        if let Some(shell_cmd) = shell_command {
            cmd = cmd.arg(shell_cmd);
        }

        let new_pane_id = cmd
            .run_and_capture_stdout()
            .context("Failed to split pane")?;

        Ok(new_pane_id.trim().to_string())
    }
}

impl Multiplexer for TmuxBackend {
    fn name(&self) -> &'static str {
        "tmux"
    }

    // === Server/Session ===

    fn is_running(&self) -> Result<bool> {
        Cmd::new("tmux").arg("has-session").run_as_check()
    }

    fn current_pane_id(&self) -> Option<String> {
        std::env::var("TMUX_PANE").ok()
    }

    fn get_client_active_pane_path(&self) -> Result<PathBuf> {
        let output = Cmd::new("sh")
            .args(&[
                "-c",
                "tmux display-message -p -t \"$(tmux display-message -p '#{client_session}')\" '#{pane_current_path}'",
            ])
            .run_and_capture_stdout()
            .context("Failed to get client active pane path")?;

        let path = output.trim();
        if path.is_empty() {
            return Err(anyhow!("Empty path returned from tmux"));
        }

        Ok(PathBuf::from(path))
    }

    // === Window/Tab Management ===

    fn create_window(&self, params: CreateWindowParams) -> Result<String> {
        let prefixed_name = util::prefixed(params.prefix, params.name);
        let working_dir_str = params
            .cwd
            .to_str()
            .ok_or_else(|| anyhow!("Working directory path contains non-UTF8 characters"))?;

        let mut cmd = Cmd::new("tmux").args(&["new-window", "-d"]);

        // Insert after the target window if specified (keeps workmux windows grouped)
        if let Some(target) = params.after_window {
            cmd = cmd.arg("-a").args(&["-t", target]);
        }

        // Use -P to print pane info, -F to format output to just the pane ID
        let pane_id = cmd
            .args(&[
                "-n",
                &prefixed_name,
                "-c",
                working_dir_str,
                "-P",
                "-F",
                "#{pane_id}",
            ])
            .run_and_capture_stdout()
            .context("Failed to create tmux window and get pane ID")?;

        Ok(pane_id.trim().to_string())
    }

    fn kill_window(&self, full_name: &str) -> Result<()> {
        let target = format!("={}", full_name);

        Cmd::new("tmux")
            .args(&["kill-window", "-t", &target])
            .run()
            .context("Failed to kill tmux window")?;

        Ok(())
    }

    fn schedule_window_close(&self, full_name: &str, delay: Duration) -> Result<()> {
        let delay_secs = format!("{:.3}", delay.as_secs_f64());
        let target = format!("={}", full_name);
        let escaped_target = format!("'{}'", target.replace('\'', r#"'\''"#));
        let script = format!(
            "sleep {delay}; tmux kill-window -t {target} >/dev/null 2>&1",
            delay = delay_secs,
            target = escaped_target
        );

        self.run_shell(&script)
    }

    fn select_window(&self, prefix: &str, name: &str) -> Result<()> {
        let prefixed_name = util::prefixed(prefix, name);
        let target = format!("={}", prefixed_name);

        Cmd::new("tmux")
            .args(&["select-window", "-t", &target])
            .run()
            .context("Failed to select window")?;

        Ok(())
    }

    fn window_exists(&self, prefix: &str, name: &str) -> Result<bool> {
        let prefixed_name = util::prefixed(prefix, name);
        self.window_exists_by_full_name(&prefixed_name)
    }

    fn window_exists_by_full_name(&self, full_name: &str) -> Result<bool> {
        let windows = Cmd::new("tmux")
            .args(&["list-windows", "-F", "#{window_name}"])
            .run_and_capture_stdout();

        match windows {
            Ok(output) => Ok(output.lines().any(|line| line == full_name)),
            Err(_) => Ok(false),
        }
    }

    fn current_window_name(&self) -> Result<Option<String>> {
        match Cmd::new("tmux")
            .args(&["display-message", "-p", "#{window_name}"])
            .run_and_capture_stdout()
        {
            Ok(name) => Ok(Some(name.trim().to_string())),
            Err(_) => Ok(None),
        }
    }

    fn get_all_window_names(&self) -> Result<HashSet<String>> {
        let windows = Cmd::new("tmux")
            .args(&["list-windows", "-F", "#{window_name}"])
            .run_and_capture_stdout()
            .unwrap_or_default();

        Ok(windows.lines().map(String::from).collect())
    }

    fn filter_active_windows(&self, windows: &[String]) -> Result<Vec<String>> {
        let all_current = self.get_all_window_names()?;

        Ok(windows
            .iter()
            .filter(|w| all_current.contains(*w))
            .cloned()
            .collect())
    }

    fn find_last_window_with_prefix(&self, prefix: &str) -> Result<Option<String>> {
        let output = Cmd::new("tmux")
            .args(&["list-windows", "-F", "#{window_id} #{window_name}"])
            .run_and_capture_stdout()
            .unwrap_or_default();

        let mut last_match: Option<String> = None;

        for line in output.lines() {
            if let Some((id, name)) = line.split_once(' ')
                && name.starts_with(prefix)
            {
                last_match = Some(id.to_string());
            }
        }

        Ok(last_match)
    }

    fn find_last_window_with_base_handle(
        &self,
        prefix: &str,
        base_handle: &str,
    ) -> Result<Option<String>> {
        let output = Cmd::new("tmux")
            .args(&["list-windows", "-F", "#{window_id} #{window_name}"])
            .run_and_capture_stdout()
            .unwrap_or_default();

        let full_base = util::prefixed(prefix, base_handle);
        let full_base_dash = format!("{}-", full_base);
        let mut last_match: Option<String> = None;

        for line in output.lines() {
            if let Some((id, name)) = line.split_once(' ') {
                let is_exact = name == full_base;
                let is_numeric_suffix = name.strip_prefix(&full_base_dash).is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
                });

                if is_exact || is_numeric_suffix {
                    last_match = Some(id.to_string());
                }
            }
        }

        Ok(last_match)
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

    fn navigate_and_close_window(
        &self,
        prefix: &str,
        target_name: &str,
        source_name: &str,
        delay: Duration,
        trash_path: Option<&Path>,
    ) -> Result<()> {
        let delay_secs = format!("{:.3}", delay.as_secs_f64());
        let target_full = util::prefixed(prefix, target_name);
        let source_full = util::prefixed(prefix, source_name);

        // Use exact match syntax (=name) for tmux
        let target_spec = format!("={}", target_full);
        let source_spec = format!("={}", source_full);
        let target_escaped = util::shell_escape(&target_spec);
        let source_escaped = util::shell_escape(&source_spec);

        // Append trash deletion if requested
        let trash_removal = trash_path
            .map(|tp| format!("; rm -rf {}", util::shell_escape(&tp.to_string_lossy())))
            .unwrap_or_default();

        // Use tmux run-shell to execute all operations atomically in one script
        let script = format!(
            "sleep {delay}; tmux select-window -t {target} >/dev/null 2>&1; tmux kill-window -t {source} >/dev/null 2>&1{trash_removal}",
            delay = delay_secs,
            target = target_escaped,
            source = source_escaped,
            trash_removal = trash_removal,
        );

        Cmd::new("tmux")
            .args(&["run-shell", &script])
            .run()
            .context("Failed to schedule navigation and window close")?;

        Ok(())
    }

    // === Pane Management ===

    fn select_pane(&self, pane_id: &str) -> Result<()> {
        Cmd::new("tmux")
            .args(&["select-pane", "-t", pane_id])
            .run()
            .context("Failed to select pane")?;

        Ok(())
    }

    fn switch_to_pane(&self, pane_id: &str) -> Result<()> {
        Cmd::new("tmux")
            .args(&["switch-client", "-t", pane_id])
            .run()
            .context("Failed to switch to pane")?;
        Ok(())
    }

    fn respawn_pane(&self, pane_id: &str, cwd: &Path, cmd: Option<&str>) -> Result<String> {
        let working_dir_str = cwd
            .to_str()
            .ok_or_else(|| anyhow!("Working directory path contains non-UTF8 characters"))?;

        let mut command =
            Cmd::new("tmux").args(&["respawn-pane", "-t", pane_id, "-c", working_dir_str, "-k"]);

        if let Some(shell_cmd) = cmd {
            command = command.arg(shell_cmd);
        }

        command.run().context("Failed to respawn pane")?;

        // tmux respawn-pane keeps the same pane_id
        Ok(pane_id.to_string())
    }

    fn capture_pane(&self, pane_id: &str, lines: u16) -> Option<String> {
        let start_line = format!("-{}", lines);
        let output = Cmd::new("tmux")
            .args(&["capture-pane", "-p", "-e", "-S", &start_line, "-t", pane_id])
            .run_and_capture_stdout()
            .ok()?;

        Some(output)
    }

    // === Text I/O ===

    fn send_keys(&self, pane_id: &str, command: &str) -> Result<()> {
        Cmd::new("tmux")
            .args(&["send-keys", "-t", pane_id, "-l", command])
            .run()
            .context("Failed to send keys to pane")?;

        Cmd::new("tmux")
            .args(&["send-keys", "-t", pane_id, "Enter"])
            .run()
            .context("Failed to send Enter key to pane")?;

        Ok(())
    }

    fn send_keys_to_agent(&self, pane_id: &str, command: &str, agent: Option<&str>) -> Result<()> {
        if util::is_claude_agent(agent) && command.starts_with('!') {
            // Send ! first
            Cmd::new("tmux")
                .args(&["send-keys", "-t", pane_id, "-l", "!"])
                .run()
                .context("Failed to send ! to pane")?;

            // Small delay to let Claude register the !
            thread::sleep(Duration::from_millis(50));

            // Send the rest of the command
            Cmd::new("tmux")
                .args(&["send-keys", "-t", pane_id, "-l", &command[1..]])
                .run()
                .context("Failed to send keys to pane")?;

            // Send Enter
            Cmd::new("tmux")
                .args(&["send-keys", "-t", pane_id, "Enter"])
                .run()
                .context("Failed to send Enter key to pane")?;

            Ok(())
        } else {
            self.send_keys(pane_id, command)
        }
    }

    fn send_key(&self, pane_id: &str, key: &str) -> Result<()> {
        Cmd::new("tmux")
            .args(&["send-keys", "-t", pane_id, key])
            .run()
            .context("Failed to send key to pane")?;
        Ok(())
    }

    fn paste_multiline(&self, pane_id: &str, content: &str) -> Result<()> {
        use std::io::Write;

        let mut child = std::process::Command::new("tmux")
            .args(["load-buffer", "-"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("Failed to spawn tmux load-buffer")?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(content.as_bytes())
                .context("Failed to write to tmux buffer")?;
        }

        let status = child
            .wait()
            .context("Failed to wait for tmux load-buffer")?;
        if !status.success() {
            return Err(anyhow::anyhow!("tmux load-buffer failed"));
        }

        Cmd::new("tmux")
            .args(&["paste-buffer", "-t", pane_id, "-p", "-d"])
            .run()
            .context("Failed to paste buffer to pane")?;

        Cmd::new("tmux")
            .args(&["send-keys", "-t", pane_id, "Enter"])
            .run()
            .context("Failed to send Enter after paste")?;

        Ok(())
    }

    // === Shell ===

    fn get_default_shell(&self) -> Result<String> {
        self.get_default_shell_internal()
    }

    fn create_handshake(&self) -> Result<Box<dyn PaneHandshake>> {
        Ok(Box::new(TmuxHandshake::new()?))
    }

    // === Status ===

    fn set_status(&self, pane_id: &str, icon: &str, _exit_detection: bool) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let now_str = now.to_string();

        // Set Window Option for tmux status bar display.
        // Agent state is stored in filesystem (StateStore), these window options
        // are view-layer only for visual feedback in the status bar.
        if let Err(e) = Cmd::new("tmux")
            .args(&["set-option", "-w", "-t", pane_id, "@workmux_status", icon])
            .run()
        {
            eprintln!("workmux: failed to set window status: {}", e);
        }
        let _ = Cmd::new("tmux")
            .args(&[
                "set-option",
                "-w",
                "-t",
                pane_id,
                "@workmux_status_ts",
                &now_str,
            ])
            .run();

        Ok(())
    }

    fn clear_status(&self, pane_id: &str) -> Result<()> {
        self.clear_window_status_internal(pane_id);
        Ok(())
    }

    fn ensure_status_format(&self, pane_id: &str) -> Result<()> {
        self.update_format_option(pane_id, "window-status-format")?;
        self.update_format_option(pane_id, "window-status-current-format")?;
        Ok(())
    }

    // === Pane Setup ===

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

        // Handle the first pane (initial pane from window creation)
        if let Some(pane_config) = panes.first() {
            let command_to_run = if pane_config.command.as_deref() == Some("<agent>") {
                effective_agent.map(|agent_cmd| agent_cmd.to_string())
            } else {
                pane_config.command.clone()
            };

            let adjusted_command = if options.run_commands {
                command_to_run.as_ref().map(|cmd| {
                    util::adjust_command(
                        cmd,
                        options.prompt_file_path,
                        working_dir,
                        effective_agent,
                        &shell,
                    )
                })
            } else {
                None
            };

            if let Some(cmd_str) = adjusted_command.as_ref().map(|c| c.as_ref()) {
                let handshake = self.create_handshake()?;
                let wrapper = handshake.wrapper_command(&shell);

                self.respawn_pane(initial_pane_id, working_dir, Some(&wrapper))?;
                handshake.wait()?;
                self.send_keys(initial_pane_id, cmd_str)?;

                if let Some(Cow::Owned(_)) = &adjusted_command
                    && util::agent_needs_auto_status(effective_agent)
                {
                    let _ = self.set_pane_working_status(initial_pane_id, config);
                }
            }
            if pane_config.focus {
                focus_pane_id = Some(initial_pane_id.to_string());
            }
        }

        // Create additional panes by splitting
        for pane_config in panes.iter().skip(1) {
            if let Some(ref direction) = pane_config.split {
                let target_pane_idx = pane_config.target.unwrap_or(pane_ids.len() - 1);
                let target_pane_id = pane_ids
                    .get(target_pane_idx)
                    .ok_or_else(|| anyhow!("Invalid target pane index: {}", target_pane_idx))?;

                let command_to_run = if pane_config.command.as_deref() == Some("<agent>") {
                    effective_agent.map(|agent_cmd| agent_cmd.to_string())
                } else {
                    pane_config.command.clone()
                };

                let adjusted_command = if options.run_commands {
                    command_to_run.as_ref().map(|cmd| {
                        util::adjust_command(
                            cmd,
                            options.prompt_file_path,
                            working_dir,
                            effective_agent,
                            &shell,
                        )
                    })
                } else {
                    None
                };

                let new_pane_id =
                    if let Some(cmd_str) = adjusted_command.as_ref().map(|c| c.as_ref()) {
                        let handshake = self.create_handshake()?;
                        let wrapper = handshake.wrapper_command(&shell);

                        let pane_id = self.split_pane_internal(
                            target_pane_id,
                            direction,
                            working_dir,
                            pane_config.size,
                            pane_config.percentage,
                            Some(&wrapper),
                        )?;

                        handshake.wait()?;
                        self.send_keys(&pane_id, cmd_str)?;

                        if let Some(Cow::Owned(_)) = &adjusted_command
                            && util::agent_needs_auto_status(effective_agent)
                        {
                            let _ = self.set_pane_working_status(&pane_id, config);
                        }

                        pane_id
                    } else {
                        self.split_pane_internal(
                            target_pane_id,
                            direction,
                            working_dir,
                            pane_config.size,
                            pane_config.percentage,
                            None,
                        )?
                    };

                if pane_config.focus {
                    focus_pane_id = Some(new_pane_id.clone());
                }
                pane_ids.push(new_pane_id);
            }
        }

        Ok(PaneSetupResult {
            focus_pane_id: focus_pane_id.unwrap_or_else(|| initial_pane_id.to_string()),
        })
    }

    // === State Reconciliation ===

    fn instance_id(&self) -> String {
        // Check TMUX environment variable for socket path
        // Format: /path/to/socket,pid,session_index
        std::env::var("TMUX")
            .ok()
            .and_then(|tmux| tmux.split(',').next().map(String::from))
            .unwrap_or_else(|| "default".to_string())
    }

    fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>> {
        let format = "#{pane_id}\t#{pane_pid}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_title}\t#{session_name}\t#{window_name}";

        // Use display-message to query a specific pane
        let output = Cmd::new("tmux")
            .args(&["display-message", "-t", pane_id, "-p", format])
            .run_and_capture_stdout();

        let output = match output {
            Ok(o) => o,
            Err(_) => return Ok(None), // Pane doesn't exist or error querying
        };

        let line = output.trim();
        if line.is_empty() {
            return Ok(None);
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 7 {
            return Ok(None);
        }

        Ok(Some(LivePaneInfo {
            pid: parts[1].parse().unwrap_or(0),
            current_command: parts[2].to_string(),
            working_dir: PathBuf::from(parts[3]),
            title: if parts[4].is_empty() {
                None
            } else {
                Some(parts[4].to_string())
            },
            session: Some(parts[5].to_string()),
            window: Some(parts[6].to_string()),
        }))
    }
}

/// Format string to inject into tmux window-status-format.
const WORKMUX_STATUS_FORMAT: &str = "#{?@workmux_status, #{@workmux_status},}";

/// Injects workmux status format into an existing format string.
fn inject_status_format(format: &str) -> String {
    let patterns = ["#{window_flags", "#{?window_flags", "#{F}"];
    let insert_pos = patterns.iter().filter_map(|p| format.find(p)).min();

    if let Some(pos) = insert_pos {
        let (before, after) = format.split_at(pos);
        format!("{}{}{}", before, WORKMUX_STATUS_FORMAT, after)
    } else {
        format!("{}{}", format, WORKMUX_STATUS_FORMAT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inject_status_format_standard() {
        let input = "#I:#W#{?window_flags,#{window_flags}, }";
        let result = inject_status_format(input);
        assert_eq!(
            result,
            "#I:#W#{?@workmux_status, #{@workmux_status},}#{?window_flags,#{window_flags}, }"
        );
    }

    #[test]
    fn test_inject_status_format_short_flags() {
        let input = "#I:#W#{F}";
        let result = inject_status_format(input);
        assert_eq!(result, "#I:#W#{?@workmux_status, #{@workmux_status},}#{F}");
    }

    #[test]
    fn test_inject_status_format_no_flags() {
        let input = "#I:#W";
        let result = inject_status_format(input);
        assert_eq!(result, "#I:#W#{?@workmux_status, #{@workmux_status},}");
    }

    #[test]
    fn test_inject_status_format_complex() {
        let input = "#[fg=blue]#I#[default] #{?window_flags,#{window_flags},}";
        let result = inject_status_format(input);
        assert_eq!(
            result,
            "#[fg=blue]#I#[default] #{?@workmux_status, #{@workmux_status},}#{?window_flags,#{window_flags},}"
        );
    }
}
