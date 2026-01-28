//! tmux backend implementation for the Multiplexer trait.
//!
//! This module provides TmuxBackend, which wraps all tmux-specific operations
//! and exposes them through the Multiplexer trait interface.

use anyhow::{Context, Result, anyhow};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use crate::cmd::Cmd;
use crate::config::SplitDirection as ConfigSplitDirection;

use super::handshake::TmuxHandshake;
use super::types::*;
use super::{Multiplexer, PaneHandshake, agent, util};

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

    /// Run a tmux command, returning an error with context on failure.
    fn tmux_cmd(&self, args: &[&str]) -> Result<()> {
        Cmd::new("tmux")
            .args(args)
            .run()
            .with_context(|| format!("tmux command failed: {:?}", args))?;
        Ok(())
    }

    /// Run a tmux command and capture stdout.
    fn tmux_query(&self, args: &[&str]) -> Result<String> {
        Cmd::new("tmux")
            .args(args)
            .run_and_capture_stdout()
            .with_context(|| format!("tmux query failed: {:?}", args))
    }

    /// Get the default shell configured in tmux.
    fn get_default_shell_internal(&self) -> Result<String> {
        let output = self.tmux_query(&["show-option", "-gqv", "default-shell"])?;
        let shell = output.trim();
        if shell.is_empty() {
            Ok("/bin/bash".to_string())
        } else {
            Ok(shell.to_string())
        }
    }

    /// Execute a shell script via tmux run-shell.
    fn run_shell(&self, script: &str) -> Result<()> {
        self.tmux_cmd(&["run-shell", script])
    }

    /// Clear the window status display (status bar icon).
    fn clear_window_status_internal(&self, pane_id: &str) {
        let _ = self.tmux_cmd(&["set-option", "-uw", "-t", pane_id, "@workmux_status"]);
    }

    /// Updates a single tmux format option for the target window to include workmux status.
    fn update_format_option(&self, pane: &str, option: &str) -> Result<()> {
        // Read current format. Try window-level first, fall back to global.
        //
        // Uses run() instead of tmux_query()/run_and_capture_stdout() because the latter
        // calls .trim() which strips meaningful whitespace from format strings (e.g.,
        // padding spaces in tmux themes). We only strip trailing newlines from command output.
        let window_format = Cmd::new("tmux")
            .args(&["show-option", "-wv", "-t", pane, option])
            .run()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|s| s.trim_end_matches('\n').to_string())
            .filter(|s| !s.is_empty());

        let current = match window_format {
            Some(fmt) => fmt,
            None => Cmd::new("tmux")
                .args(&["show-option", "-gv", option])
                .run()
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|s| s.trim_end_matches('\n').to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "#I:#W#{?window_flags,#{window_flags}, }".to_string()),
        };

        if !current.contains("@workmux_status") {
            let new_format = inject_status_format(&current);
            // Set per-window to avoid affecting other windows/sessions
            self.tmux_cmd(&["set-option", "-w", "-t", pane, option, &new_format])?;
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

        // Wrap in sh -c "..." to ensure POSIX evaluation even when tmux's
        // default-shell is a non-POSIX shell like nushell.
        let wrapped;
        if let Some(script) = shell_command {
            wrapped = format!("sh -c \"{}\"", util::escape_for_double_quotes(script));
            cmd = cmd.arg(&wrapped);
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

    fn active_pane_id(&self) -> Option<String> {
        self.tmux_query(&["display-message", "-p", "#{pane_id}"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
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

    fn create_session(&self, params: CreateSessionParams) -> Result<String> {
        let prefixed_name = util::prefixed(params.prefix, params.name);
        let working_dir_str = params
            .cwd
            .to_str()
            .ok_or_else(|| anyhow!("Working directory path contains non-UTF8 characters"))?;

        // Create a new detached session with the specified name and working directory
        // -d: detached (don't switch to it yet)
        // -s: session name
        // -c: start directory
        // -P -F: print the pane ID of the initial window
        let pane_id = Cmd::new("tmux")
            .args(&[
                "new-session",
                "-d",
                "-s",
                &prefixed_name,
                "-c",
                working_dir_str,
                "-P",
                "-F",
                "#{pane_id}",
            ])
            .run_and_capture_stdout()
            .context("Failed to create tmux session and get pane ID")?;

        Ok(pane_id.trim().to_string())
    }

    fn switch_to_session(&self, prefix: &str, name: &str) -> Result<()> {
        let prefixed_name = util::prefixed(prefix, name);
        self.tmux_cmd(&["switch-client", "-t", &prefixed_name])
    }

    fn kill_window(&self, full_name: &str) -> Result<()> {
        let target = format!("={}", full_name);
        self.tmux_cmd(&["kill-window", "-t", &target])
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

    fn run_deferred_script(&self, script: &str) -> Result<()> {
        self.run_shell(script)
    }

    fn shell_select_window_cmd(&self, full_name: &str) -> Result<String> {
        let session = self.current_session().unwrap_or_default();
        let session_prefix = if session.is_empty() {
            String::new()
        } else {
            format!("{}:", session)
        };
        let target = format!("{}={}", session_prefix, full_name);
        let escaped = format!("'{}'", target.replace('\'', r#"'\''"#));
        Ok(format!("tmux select-window -t {} >/dev/null 2>&1", escaped))
    }

    fn shell_kill_window_cmd(&self, full_name: &str) -> Result<String> {
        let session = self.current_session().unwrap_or_default();
        let session_prefix = if session.is_empty() {
            String::new()
        } else {
            format!("{}:", session)
        };
        let target = format!("{}={}", session_prefix, full_name);
        let escaped = format!("'{}'", target.replace('\'', r#"'\''"#));
        Ok(format!("tmux kill-window -t {} >/dev/null 2>&1", escaped))
    }

    fn select_window(&self, prefix: &str, name: &str) -> Result<()> {
        let prefixed_name = util::prefixed(prefix, name);
        let target = format!("={}", prefixed_name);
        self.tmux_cmd(&["select-window", "-t", &target])
    }

    fn window_exists(&self, prefix: &str, name: &str) -> Result<bool> {
        let prefixed_name = util::prefixed(prefix, name);
        self.window_exists_by_full_name(&prefixed_name)
    }

    fn window_exists_by_full_name(&self, full_name: &str) -> Result<bool> {
        match self.tmux_query(&["list-windows", "-F", "#{window_name}"]) {
            Ok(output) => Ok(output.lines().any(|line| line == full_name)),
            Err(_) => Ok(false),
        }
    }

    fn current_window_name(&self) -> Result<Option<String>> {
        match self.tmux_query(&["display-message", "-p", "#{window_name}"]) {
            Ok(name) => Ok(Some(name.trim().to_string())),
            Err(_) => Ok(None),
        }
    }

    fn current_session(&self) -> Option<String> {
        self.tmux_query(&["display-message", "-p", "#{session_name}"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    fn get_all_window_names(&self) -> Result<HashSet<String>> {
        let windows = self
            .tmux_query(&["list-windows", "-F", "#{window_name}"])
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
        let output = self
            .tmux_query(&["list-windows", "-F", "#{window_id} #{window_name}"])
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
        let output = self
            .tmux_query(&["list-windows", "-F", "#{window_id} #{window_name}"])
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

    // === Pane Management ===

    fn select_pane(&self, pane_id: &str) -> Result<()> {
        self.tmux_cmd(&["select-pane", "-t", pane_id])
    }

    fn switch_to_pane(&self, pane_id: &str) -> Result<()> {
        self.tmux_cmd(&["switch-client", "-t", pane_id])
    }

    fn respawn_pane(&self, pane_id: &str, cwd: &Path, cmd: Option<&str>) -> Result<String> {
        let working_dir_str = cwd
            .to_str()
            .ok_or_else(|| anyhow!("Working directory path contains non-UTF8 characters"))?;

        let mut command =
            Cmd::new("tmux").args(&["respawn-pane", "-t", pane_id, "-c", working_dir_str, "-k"]);

        // Wrap in sh -c "..." to ensure POSIX evaluation even when tmux's
        // default-shell is a non-POSIX shell like nushell.
        let wrapped;
        if let Some(script) = cmd {
            wrapped = format!("sh -c \"{}\"", util::escape_for_double_quotes(script));
            command = command.arg(&wrapped);
        }

        command.run().context("Failed to respawn pane")?;

        // tmux respawn-pane keeps the same pane_id
        Ok(pane_id.to_string())
    }

    fn capture_pane(&self, pane_id: &str, lines: u16) -> Option<String> {
        let start_line = format!("-{}", lines);
        self.tmux_query(&["capture-pane", "-p", "-e", "-S", &start_line, "-t", pane_id])
            .ok()
    }

    // === Text I/O ===

    fn send_keys(&self, pane_id: &str, command: &str) -> Result<()> {
        self.tmux_cmd(&["send-keys", "-t", pane_id, "-l", command])?;
        self.tmux_cmd(&["send-keys", "-t", pane_id, "Enter"])
    }

    fn send_keys_to_agent(&self, pane_id: &str, command: &str, agent: Option<&str>) -> Result<()> {
        if agent::resolve_profile(agent).needs_bang_delay() && command.starts_with('!') {
            // Send ! first
            self.tmux_cmd(&["send-keys", "-t", pane_id, "-l", "!"])?;

            // Small delay to let Claude register the !
            thread::sleep(Duration::from_millis(50));

            // Send the rest of the command
            self.tmux_cmd(&["send-keys", "-t", pane_id, "-l", &command[1..]])?;

            // Send Enter
            self.tmux_cmd(&["send-keys", "-t", pane_id, "Enter"])
        } else {
            self.send_keys(pane_id, command)
        }
    }

    fn send_key(&self, pane_id: &str, key: &str) -> Result<()> {
        self.tmux_cmd(&["send-keys", "-t", pane_id, key])
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

        self.tmux_cmd(&["paste-buffer", "-t", pane_id, "-p", "-d"])?;
        self.tmux_cmd(&["send-keys", "-t", pane_id, "Enter"])
    }

    // === Shell ===

    fn get_default_shell(&self) -> Result<String> {
        self.get_default_shell_internal()
    }

    fn create_handshake(&self) -> Result<Box<dyn PaneHandshake>> {
        Ok(Box::new(TmuxHandshake::new()?))
    }

    // === Status ===

    fn set_status(&self, pane_id: &str, icon: &str, auto_clear_on_focus: bool) -> Result<()> {
        // Set Window Option for tmux status bar display.
        // Agent state is stored in filesystem (StateStore), these window options
        // are view-layer only for visual feedback in the status bar.
        if let Err(e) = self.tmux_cmd(&["set-option", "-w", "-t", pane_id, "@workmux_status", icon])
        {
            eprintln!("workmux: failed to set window status: {}", e);
        }

        // Set up hook to auto-clear status when window receives focus.
        // Used for "waiting" and "done" statuses so they clear once the user sees them.
        if auto_clear_on_focus {
            // Only clear if status still matches this icon (avoids clearing a newer status)
            let hook_cmd = format!(
                "if-shell -F \"#{{==:#{{@workmux_status}},{}}}\" \"set-option -uw @workmux_status\"",
                icon
            );
            let _ = self.tmux_cmd(&["set-hook", "-w", "-t", pane_id, "pane-focus-in", &hook_cmd]);
        }

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

    fn split_pane(
        &self,
        target_pane_id: &str,
        direction: &crate::config::SplitDirection,
        cwd: &Path,
        size: Option<u16>,
        percentage: Option<u8>,
        command: Option<&str>,
    ) -> Result<String> {
        self.split_pane_internal(target_pane_id, direction, cwd, size, percentage, command)
    }

    // === State Reconciliation ===

    fn instance_id(&self) -> String {
        // TMUX env var format: /path/to/socket,pid,session_index
        // We use only the socket path, which identifies the tmux server.
        // All sessions on the same server share one socket, so instance_id
        // is per-server, not per-session.
        std::env::var("TMUX")
            .ok()
            .and_then(|tmux| tmux.split(',').next().map(String::from))
            .unwrap_or_else(|| "default".to_string())
    }

    fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>> {
        let format = "#{pane_id}\t#{pane_pid}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_title}\t#{session_name}\t#{window_name}";

        // Use display-message to query a specific pane
        let output = self.tmux_query(&["display-message", "-t", pane_id, "-p", format]);

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

    fn get_all_live_pane_info(&self) -> Result<std::collections::HashMap<String, LivePaneInfo>> {
        use std::collections::HashMap;

        let format = "#{pane_id}\t#{pane_pid}\t#{pane_current_command}\t#{pane_current_path}\t#{pane_title}\t#{session_name}\t#{window_name}";

        // Use list-panes -a to query ALL panes across all sessions at once
        let output = self.tmux_query(&["list-panes", "-a", "-F", format])?;

        let mut panes = HashMap::new();

        for line in output.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() < 7 {
                continue;
            }

            let pane_id = parts[0].to_string();
            panes.insert(
                pane_id,
                LivePaneInfo {
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
                },
            );
        }

        Ok(panes)
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

    #[test]
    fn test_inject_status_format_preserves_whitespace() {
        // Leading and trailing spaces from tmux themes must be preserved
        let input = " #I:#W#{window_flags} ";
        let result = inject_status_format(input);
        assert_eq!(
            result,
            " #I:#W#{?@workmux_status, #{@workmux_status},}#{window_flags} "
        );
    }

    #[test]
    fn test_trim_end_newlines_preserves_spaces() {
        // Simulates processing tmux show-option output: trailing newlines are
        // stripped but meaningful whitespace (padding spaces) is kept intact.
        let raw_output = " #I:#W#{window_flags} \n";
        let processed = raw_output.trim_end_matches('\n').to_string();
        assert_eq!(processed, " #I:#W#{window_flags} ");

        let result = inject_status_format(&processed);
        assert_eq!(
            result,
            " #I:#W#{?@workmux_status, #{@workmux_status},}#{window_flags} "
        );
    }
}
