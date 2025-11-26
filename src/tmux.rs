use anyhow::{Context, Result, anyhow};
use std::borrow::Cow;
use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use crate::cmd::Cmd;
use crate::config::{PaneConfig, SplitDirection};

/// Helper function to add prefix to window name
pub fn prefixed(prefix: &str, window_name: &str) -> String {
    format!("{}{}", prefix, window_name)
}

/// Get all tmux window names in a single call
pub fn get_all_window_names() -> Result<HashSet<String>> {
    // tmux list-windows may exit with error if no windows exist
    let windows = Cmd::new("tmux")
        .args(&["list-windows", "-F", "#{window_name}"])
        .run_and_capture_stdout()
        .unwrap_or_default(); // Return empty string if command fails

    Ok(windows.lines().map(String::from).collect())
}

/// Check if tmux server is running
pub fn is_running() -> Result<bool> {
    Cmd::new("tmux").arg("has-session").run_as_check()
}

/// Check if a tmux window with the given name exists
pub fn window_exists(prefix: &str, window_name: &str) -> Result<bool> {
    let prefixed_name = prefixed(prefix, window_name);
    let windows = Cmd::new("tmux")
        .args(&["list-windows", "-F", "#{window_name}"])
        .run_and_capture_stdout();

    match windows {
        Ok(output) => Ok(output.lines().any(|line| line == prefixed_name)),
        Err(_) => Ok(false), // If command fails, window doesn't exist
    }
}

/// Return the tmux window name for the current pane, if any
pub fn current_window_name() -> Result<Option<String>> {
    match Cmd::new("tmux")
        .args(&["display-message", "-p", "#{window_name}"])
        .run_and_capture_stdout()
    {
        Ok(name) => Ok(Some(name.trim().to_string())),
        Err(_) => Ok(None),
    }
}

/// Create a new tmux window with the given name and working directory.
/// Returns the pane ID of the initial pane in the window.
pub fn create_window(
    prefix: &str,
    window_name: &str,
    working_dir: &Path,
    detached: bool,
) -> Result<String> {
    let prefixed_name = prefixed(prefix, window_name);
    let working_dir_str = working_dir
        .to_str()
        .ok_or_else(|| anyhow!("Working directory path contains non-UTF8 characters"))?;

    let mut cmd = Cmd::new("tmux").arg("new-window");
    if detached {
        cmd = cmd.arg("-d");
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

/// Select a specific pane by its ID
pub fn select_pane(pane_id: &str) -> Result<()> {
    Cmd::new("tmux")
        .args(&["select-pane", "-t", pane_id])
        .run()
        .context("Failed to select pane")?;

    Ok(())
}

/// Select a specific window
pub fn select_window(prefix: &str, window_name: &str) -> Result<()> {
    let prefixed_name = prefixed(prefix, window_name);
    let target = format!("={}", prefixed_name);

    Cmd::new("tmux")
        .args(&["select-window", "-t", &target])
        .run()
        .context("Failed to select window")?;

    Ok(())
}

/// Kill a tmux window
pub fn kill_window(prefix: &str, window_name: &str) -> Result<()> {
    let prefixed_name = prefixed(prefix, window_name);
    let target = format!("={}", prefixed_name);

    Cmd::new("tmux")
        .args(&["kill-window", "-t", &target])
        .run()
        .context("Failed to kill tmux window")?;

    Ok(())
}

/// Execute a shell script via tmux run-shell
pub fn run_shell(script: &str) -> Result<()> {
    Cmd::new("tmux")
        .args(&["run-shell", script])
        .run()
        .context("Failed to run shell command via tmux")?;
    Ok(())
}

/// Schedule a tmux window to be killed after a short delay. This is useful when
/// the current command is running inside the window that needs to close.
pub fn schedule_window_close(prefix: &str, window_name: &str, delay: Duration) -> Result<()> {
    let prefixed_name = prefixed(prefix, window_name);
    let delay_secs = format!("{:.3}", delay.as_secs_f64());
    let script = format!(
        "sleep {delay}; tmux kill-window -t ={window} >/dev/null 2>&1",
        delay = delay_secs,
        window = prefixed_name
    );

    run_shell(&script)
}

/// Builds a shell command string for tmux that executes an optional user command
/// and then leaves an interactive shell open.
///
/// The escaping strategy uses POSIX-style quote escaping ('\'\'). This works
/// correctly with bash, zsh, fish, and other common shells.
pub fn build_startup_command(command: Option<&str>) -> Result<Option<String>> {
    let command = match command {
        Some(c) => c,
        None => return Ok(None),
    };

    let shell_path = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let shell_name = std::path::Path::new(&shell_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    // Nushell requires completely different syntax - it's not POSIX-compatible
    if shell_name == "nu" {
        return build_nushell_startup_command(command, &shell_path);
    }

    // Manually trigger shell pre-prompt hooks to ensure tools like direnv,
    // nvm, and rbenv are loaded before the user command is executed. These
    // hooks are normally only triggered before an interactive prompt.
    let pre_command_hook = match shell_name {
        "zsh" => {
            "if (( ${#precmd_functions[@]} )); then for f in \"${precmd_functions[@]}\"; do \"$f\"; done; fi"
        }
        "bash" => "eval \"${PROMPT_COMMAND:-}\"",
        "fish" => "functions -q fish_prompt; and emit fish_prompt",
        _ => "true", // No-op for other shells
    };

    // To run `user_command` and then `exec shell` inside a new shell instance,
    // we use the form: `$SHELL -ic '<hooks>; <user_command>; exec $SHELL -l'`.
    // We must escape single quotes within the user command using POSIX-style escaping.
    let escaped_command = command.replace('\'', r#"'\''"#);

    // A new pane's interactive shell can have a different `PATH` than the tmux server,
    // especially after sourcing rc files (`.zshrc`, etc.). This can lead to "command not found"
    // errors for executables that `workmux` can resolve but the pane's shell cannot.
    //
    // To ensure consistency, explicitly fetch the tmux server's global `PATH` and
    // append it to the pane's `PATH` before executing the user's command. This
    // guarantees that agents and other tools are discoverable while allowing
    // version managers (nvm, rbenv, mise, corepack) to take precedence.
    let command_prologue = crate::config::tmux_global_path().map(|tmux_path| {
        let escaped_path = tmux_path.replace('\'', r#"'\''"#);
        format!("export PATH=$PATH:'{}'; ", escaped_path)
    });

    let inner_command = format!(
        "{prologue}{pre_hook}; {user_cmd}; exec {shell} -l",
        prologue = command_prologue.as_deref().unwrap_or(""),
        pre_hook = pre_command_hook,
        user_cmd = escaped_command,
        shell = shell_path,
    );

    // The initial shell is interactive (-i) to ensure rc files (~/.bashrc,
    // ~/.zshrc) are sourced, which is where shell hooks are configured. It is
    // NOT a login shell (-l), as this can prevent rc files from sourcing in
    // bash. The final `exec $SHELL -l` ensures the user is left in a login
    // shell, matching tmux's default behavior.
    let full_command = format!(
        "{shell} -ic '{inner_command}'",
        shell = shell_path,
        inner_command = inner_command,
    );

    Ok(Some(full_command))
}

/// Builds a Nushell-compatible startup command.
///
/// Nushell is not POSIX-compatible and requires different syntax:
/// - Uses `$env.PATH` instead of `export PATH`
/// - Uses `-l -e` for "login shell, execute, then interactive"
/// - Has different quoting and escaping rules
fn build_nushell_startup_command(command: &str, shell_path: &str) -> Result<Option<String>> {
    // Build PATH append command if tmux has a global PATH set
    let path_prologue = crate::config::tmux_global_path()
        .map(|tmux_path| {
            // Escape backslashes and double quotes for Nushell string
            let escaped_path = tmux_path.replace('\\', "\\\\").replace('"', "\\\"");
            format!(r#"$env.PATH = ($env.PATH | append "{}"); "#, escaped_path)
        })
        .unwrap_or_default();

    // Escape the user command for embedding in single-quoted Nushell string
    // Single quotes in Nushell are literal strings, escape by doubling
    let escaped_command = command.replace('\'', "''");

    // Build the command to execute: append PATH, then run user command
    let inner_command = format!(
        "{path_prologue}{user_cmd}",
        path_prologue = path_prologue.replace('\'', "''"),
        user_cmd = escaped_command,
    );

    // Use -l (login) to source config files and -e (execute) to run command
    // then enter interactive mode. The -e flag keeps the shell open after
    // the command completes, which is exactly what we need.
    let full_command = format!(
        "{shell} -l -e '{inner}'",
        shell = shell_path,
        inner = inner_command,
    );

    Ok(Some(full_command))
}

/// Split a pane with optional command and return the new pane's ID
pub fn split_pane_with_command(
    target_pane_id: &str,
    direction: &SplitDirection,
    working_dir: &Path,
    command: Option<&str>,
    size: Option<u16>,
    percentage: Option<u8>,
) -> Result<String> {
    let split_arg = match direction {
        SplitDirection::Horizontal => "-h",
        SplitDirection::Vertical => "-v",
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
        "-P", // Print new pane info
        "-F", // Format to get just the ID
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

    if let Some(cmd_str) = command {
        cmd = cmd.arg(cmd_str);
    };

    let new_pane_id = cmd
        .run_and_capture_stdout()
        .context("Failed to split pane")?;

    Ok(new_pane_id.trim().to_string())
}

/// Respawn a pane with a new command by its ID
pub fn respawn_pane(pane_id: &str, working_dir: &Path, command: &str) -> Result<()> {
    let working_dir_str = working_dir
        .to_str()
        .ok_or_else(|| anyhow!("Working directory path contains non-UTF8 characters"))?;

    Cmd::new("tmux")
        .args(&[
            "respawn-pane",
            "-t",
            pane_id,
            "-c",
            working_dir_str,
            "-k",
            command,
        ])
        .run()
        .context("Failed to respawn pane")?;

    Ok(())
}

/// Result of setting up panes
pub struct PaneSetupResult {
    /// The ID of the pane that should receive focus.
    pub focus_pane_id: String,
}

pub struct PaneSetupOptions<'a> {
    pub run_commands: bool,
    pub prompt_file_path: Option<&'a Path>,
}

/// Setup panes in a window according to configuration
pub fn setup_panes(
    initial_pane_id: &str,
    panes: &[PaneConfig],
    working_dir: &Path,
    pane_options: PaneSetupOptions<'_>,
    config: &crate::config::Config,
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

    // Handle the first pane (initial pane from window creation)
    if let Some(pane_config) = panes.first() {
        let command_to_run = if pane_config.command.as_deref() == Some("<agent>") {
            effective_agent.map(|agent_cmd| agent_cmd.to_string())
        } else {
            pane_config.command.clone()
        };

        let adjusted_command = if pane_options.run_commands {
            command_to_run.as_ref().map(|cmd| {
                adjust_command(
                    cmd,
                    pane_options.prompt_file_path,
                    working_dir,
                    effective_agent,
                )
            })
        } else {
            None
        };

        if let Some(cmd_str) = adjusted_command.as_ref().map(|c| c.as_ref())
            && let Some(startup_cmd) = build_startup_command(Some(cmd_str))?
        {
            respawn_pane(initial_pane_id, working_dir, &startup_cmd)?;
        }
        if pane_config.focus {
            focus_pane_id = Some(initial_pane_id.to_string());
        }
    }

    // Create additional panes by splitting
    for pane_config in panes.iter().skip(1) {
        if let Some(ref direction) = pane_config.split {
            // Determine which pane to split based on logical index, then get its ID
            let target_pane_idx = pane_config.target.unwrap_or(pane_ids.len() - 1);
            let target_pane_id = pane_ids
                .get(target_pane_idx)
                .ok_or_else(|| anyhow!("Invalid target pane index: {}", target_pane_idx))?;

            let command_to_run = if pane_config.command.as_deref() == Some("<agent>") {
                effective_agent.map(|agent_cmd| agent_cmd.to_string())
            } else {
                pane_config.command.clone()
            };

            let adjusted_command = if pane_options.run_commands {
                command_to_run.as_ref().map(|cmd| {
                    adjust_command(
                        cmd,
                        pane_options.prompt_file_path,
                        working_dir,
                        effective_agent,
                    )
                })
            } else {
                None
            };

            let startup_cmd = build_startup_command(adjusted_command.as_ref().map(|c| c.as_ref()))?;

            let new_pane_id = split_pane_with_command(
                target_pane_id,
                direction,
                working_dir,
                startup_cmd.as_deref(),
                pane_config.size,
                pane_config.percentage,
            )?;

            if pane_config.focus {
                focus_pane_id = Some(new_pane_id.clone());
            }
            pane_ids.push(new_pane_id);
        }
    }

    Ok(PaneSetupResult {
        // Default to the first pane if no focus is specified
        focus_pane_id: focus_pane_id.unwrap_or_else(|| initial_pane_id.to_string()),
    })
}

fn adjust_command<'a>(
    command: &'a str,
    prompt_file_path: Option<&Path>,
    working_dir: &Path,
    effective_agent: Option<&str>,
) -> Cow<'a, str> {
    if let Some(prompt_path) = prompt_file_path
        && let Some(rewritten) =
            rewrite_agent_command(command, prompt_path, working_dir, effective_agent)
    {
        return Cow::Owned(rewritten);
    }
    Cow::Borrowed(command)
}

/// Rewrites an agent command to inject a prompt file's contents.
///
/// When a prompt file is provided (via --prompt-file or --prompt-editor), this function
/// modifies the agent command to automatically pass the prompt content. For example,
/// "claude" becomes "claude \"$(cat PROMPT.md)\"".
///
/// Only rewrites commands that match the configured agent. For instance, if the config
/// specifies "gemini" as the agent, a "claude" command won't be rewritten.
///
/// Special handling:
/// - gemini: Adds `-i` flag for interactive mode after the prompt
/// - Other agents (claude, codex, etc.): Just passes the prompt as first argument
///
/// Returns None if the command shouldn't be rewritten (empty, doesn't match configured agent, etc.)
fn rewrite_agent_command(
    command: &str,
    prompt_file: &Path,
    working_dir: &Path,
    effective_agent: Option<&str>,
) -> Option<String> {
    let agent_command = effective_agent?;
    let trimmed_command = command.trim();
    if trimmed_command.is_empty() {
        return None;
    }

    let (pane_token, pane_rest) = crate::config::split_first_token(trimmed_command)?;
    let (config_token, _) = crate::config::split_first_token(agent_command)?;

    let resolved_pane_path = crate::config::resolve_executable_path(pane_token)
        .unwrap_or_else(|| pane_token.to_string());
    let resolved_config_path = crate::config::resolve_executable_path(config_token)
        .unwrap_or_else(|| config_token.to_string());

    let pane_stem = Path::new(&resolved_pane_path).file_stem();
    let config_stem = Path::new(&resolved_config_path).file_stem();

    if pane_stem != config_stem {
        return None;
    }

    let relative = prompt_file.strip_prefix(working_dir).unwrap_or(prompt_file);
    let prompt_path = relative.to_string_lossy();
    let rest = pane_rest.trim_start();

    // Build the command step-by-step to ensure correct order:
    // [agent_command] [agent_options] [user_args] [prompt_argument]
    let mut cmd = pane_token.to_string();

    // Add user-provided arguments from config (must come before the prompt)
    if !rest.is_empty() {
        cmd.push(' ');
        cmd.push_str(rest);
    }

    // Add the prompt argument (agent-specific handling)
    let is_gemini = pane_stem.and_then(|s| s.to_str()) == Some("gemini");
    if is_gemini {
        // gemini uses -i flag with the prompt as its argument
        cmd.push_str(&format!(" -i \"$(cat {})\"", prompt_path));
    } else {
        // Other agents use -- separator
        cmd.push_str(&format!(" -- \"$(cat {})\"", prompt_path));
    }

    Some(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_rewrite_claude_command() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command("claude", &prompt_file, &working_dir, Some("claude"));
        assert_eq!(result, Some("claude -- \"$(cat PROMPT.md)\"".to_string()));
    }

    #[test]
    fn test_rewrite_codex_command() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command("codex", &prompt_file, &working_dir, Some("codex"));
        assert_eq!(result, Some("codex -- \"$(cat PROMPT.md)\"".to_string()));
    }

    #[test]
    fn test_rewrite_gemini_command() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command("gemini", &prompt_file, &working_dir, Some("gemini"));
        assert_eq!(result, Some("gemini -i \"$(cat PROMPT.md)\"".to_string()));
    }

    #[test]
    fn test_rewrite_command_with_path() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command(
            "/usr/local/bin/claude",
            &prompt_file,
            &working_dir,
            Some("/usr/local/bin/claude"),
        );
        assert_eq!(
            result,
            Some("/usr/local/bin/claude -- \"$(cat PROMPT.md)\"".to_string())
        );
    }

    #[test]
    fn test_rewrite_command_with_args() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command(
            "claude --verbose",
            &prompt_file,
            &working_dir,
            Some("claude"),
        );
        assert_eq!(
            result,
            Some("claude --verbose -- \"$(cat PROMPT.md)\"".to_string())
        );
    }

    #[test]
    fn test_rewrite_mismatched_agent() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        // Command is for claude
        let result = rewrite_agent_command("claude", &prompt_file, &working_dir, Some("gemini"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_rewrite_unknown_agent() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command(
            "unknown-agent",
            &prompt_file,
            &working_dir,
            Some("unknown-agent"),
        );
        assert_eq!(
            result,
            Some("unknown-agent -- \"$(cat PROMPT.md)\"".to_string())
        );
    }

    #[test]
    fn test_rewrite_empty_command() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command("", &prompt_file, &working_dir, Some("claude"));
        assert_eq!(result, None);
    }

    #[test]
    fn test_build_nushell_startup_command_uses_correct_flags() {
        let result = build_nushell_startup_command("echo hello", "/usr/bin/nu").unwrap();
        let cmd = result.unwrap();

        // Should use -l (login) and -e (execute then interactive) flags
        assert!(cmd.contains("-l -e"), "Expected -l -e flags, got: {}", cmd);
        assert!(
            cmd.starts_with("/usr/bin/nu"),
            "Should start with shell path"
        );
    }

    #[test]
    fn test_build_nushell_startup_command_escapes_single_quotes() {
        let result = build_nushell_startup_command("echo 'hello world'", "/usr/bin/nu").unwrap();
        let cmd = result.unwrap();

        // Nushell escapes single quotes by doubling them
        assert!(
            cmd.contains("echo ''hello world''"),
            "Expected doubled single quotes, got: {}",
            cmd
        );
    }

    #[test]
    fn test_build_nushell_startup_command_structure() {
        let result = build_nushell_startup_command("my-command", "/opt/homebrew/bin/nu").unwrap();
        let cmd = result.unwrap();

        // Should be: /path/to/nu -l -e 'command'
        assert!(
            cmd.starts_with("/opt/homebrew/bin/nu -l -e '"),
            "Expected shell -l -e 'command' format, got: {}",
            cmd
        );
        assert!(cmd.ends_with("'"), "Should end with single quote");
    }
}
