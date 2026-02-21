//! Backend-agnostic utility functions for multiplexer operations.
//!
//! These helpers are shared between tmux, WezTerm, and any future backends.

use std::borrow::Cow;
use std::path::Path;

/// Helper function to add prefix to window name.
///
/// Used by all backends to construct full window names from prefix and base name.
pub fn prefixed(prefix: &str, window_name: &str) -> String {
    format!("{}{}", prefix, window_name)
}

/// Check if a shell is POSIX-compatible (supports `$(...)` syntax).
///
/// Used to determine whether agent commands need to be wrapped in `sh -c '...'`
/// for shells like nushell or fish that don't support POSIX command substitution.
pub fn is_posix_shell(shell: &str) -> bool {
    let shell_name = Path::new(shell)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("sh");
    matches!(shell_name, "bash" | "zsh" | "sh" | "dash" | "ksh" | "ash")
}

/// Rewrites an agent command to inject a prompt file's contents.
///
/// When a prompt file is provided (via --prompt-file or --prompt-editor), this function
/// modifies the agent command to automatically pass the prompt content. For example,
/// "claude" becomes "claude -- \"$(cat PROMPT.md)\"" for POSIX shells, or wrapped in
/// `sh -c '...'` for non-POSIX shells like nushell.
///
/// Only rewrites commands that match the configured agent. For instance, if the config
/// specifies "gemini" as the agent, a "claude" command won't be rewritten.
///
/// Agent-specific prompt injection is handled via `AgentProfile::prompt_argument()`.
///
/// For non-POSIX shells (nushell, fish, pwsh), the command is wrapped in `sh -c '...'`
/// to ensure the `$(cat ...)` command substitution works correctly.
///
/// The returned command is prefixed with a space to prevent it from being saved to
/// shell history (most shells ignore commands starting with a space).
///
/// Returns None if the command shouldn't be rewritten (empty, doesn't match configured agent, etc.)
pub fn rewrite_agent_command(
    command: &str,
    prompt_file: &Path,
    working_dir: &Path,
    effective_agent: Option<&str>,
    shell: &str,
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

    // Build the inner command step-by-step to ensure correct order:
    // [agent_command] [agent_options] [user_args] [prompt_argument]
    let mut inner_cmd = pane_token.to_string();

    // Add user-provided arguments from config (must come before the prompt)
    if !rest.is_empty() {
        inner_cmd.push(' ');
        inner_cmd.push_str(rest);
    }

    // Add the prompt argument using agent profile
    let profile = super::agent::resolve_profile(effective_agent);
    inner_cmd.push(' ');
    inner_cmd.push_str(&profile.prompt_argument(&prompt_path));

    // For POSIX shells (bash, zsh, sh, etc.), use the command directly.
    // For non-POSIX shells (nushell, fish, pwsh), wrap in sh -c '...' to ensure
    // $(cat ...) command substitution works.
    // Prefix with space to prevent shell history entry.
    if is_posix_shell(shell) {
        Some(format!(" {}", inner_cmd))
    } else {
        Some(format!(" {}", wrap_for_non_posix_shell(&inner_cmd)))
    }
}

/// Resolve a pane's command: handle `<agent>` placeholder, auto-detect known
/// agents, and adjust for prompt injection.
///
/// Returns the final command to send to the pane, or None if no command should be sent.
/// This consolidates the duplicated command resolution logic from both backends' setup_panes.
/// Result of resolving a pane command.
pub struct ResolvedCommand {
    /// The command string to send to the pane.
    pub command: String,
    /// Whether the command was rewritten to inject a prompt (needs auto-status).
    pub prompt_injected: bool,
    /// The effective agent for this pane (may differ from window-level agent for auto-detected agents).
    pub effective_agent: Option<String>,
}

pub fn resolve_pane_command(
    pane_command: Option<&str>,
    run_commands: bool,
    prompt_file_path: Option<&Path>,
    working_dir: &Path,
    effective_agent: Option<&str>,
    shell: &str,
) -> Option<ResolvedCommand> {
    let raw_command = pane_command?;

    let (command, pane_effective_agent) = if raw_command == "<agent>" {
        // Bare <agent> - use window-level effective agent
        let agent = effective_agent?;
        (agent, effective_agent)
    } else if super::agent::is_known_agent(raw_command) {
        // Known agent command (e.g., "codex --flags") - use itself as effective
        // agent so prompt injection works even when it's not the configured agent
        (raw_command, Some(raw_command))
    } else {
        // Regular command - use window-level effective agent for prompt injection matching
        (raw_command, effective_agent)
    };

    if !run_commands {
        return None;
    }

    let result = adjust_command(
        command,
        prompt_file_path,
        working_dir,
        pane_effective_agent,
        shell,
    );
    let prompt_injected = matches!(result, Cow::Owned(_));
    Some(ResolvedCommand {
        command: result.into_owned(),
        prompt_injected,
        effective_agent: pane_effective_agent.map(|s| s.to_string()),
    })
}

/// Adjust a command for execution, potentially rewriting it to inject prompts.
///
/// This is a convenience wrapper around `rewrite_agent_command` that returns
/// the original command as a borrowed reference if no rewriting is needed.
pub fn adjust_command<'a>(
    command: &'a str,
    prompt_file_path: Option<&Path>,
    working_dir: &Path,
    effective_agent: Option<&str>,
    shell: &str,
) -> Cow<'a, str> {
    if let Some(prompt_path) = prompt_file_path
        && let Some(rewritten) =
            rewrite_agent_command(command, prompt_path, working_dir, effective_agent, shell)
    {
        return Cow::Owned(rewritten);
    }
    Cow::Borrowed(command)
}

/// Escape a string for embedding inside a double-quoted shell context.
///
/// Escapes: backslash, double quote, dollar sign, backtick.
/// Does NOT add surrounding quotes - caller controls the quoting.
///
/// Example: `$HOME` -> `\$HOME`
pub fn escape_for_double_quotes(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`")
}

/// Escape a command to be safely embedded inside `sh -c "..."`.
///
/// This handles the two-step nesting complexity:
/// 1. Inner single-quoted context (for paths/args inside the command)
/// 2. Outer double-quoted context (for the sh -c wrapper)
///
/// Use when you need to pass a value that will be single-quoted inside
/// a double-quoted sh -c command.
///
/// Example: `/bin/user's shell` inside `sh -c "exec '/bin/user's shell'"`:
/// - Step 1: `'\''` escaping -> `/bin/user'\''s shell`
/// - Step 2: double-quote escaping -> `/bin/user'\''s shell` (no change here)
pub fn escape_for_sh_c_inner_single_quote(s: &str) -> String {
    let single_escaped = s.replace('\'', "'\\''");
    escape_for_double_quotes(&single_escaped)
}

/// Wrap a command in `sh -c '...'` for execution in non-POSIX shells.
///
/// Used when the default shell (nushell, fish, etc.) doesn't support
/// POSIX command substitution like `$(...)`.
pub fn wrap_for_non_posix_shell(command: &str) -> String {
    let escaped = command.replace('\'', "'\\''");
    format!("sh -c '{}'", escaped)
}

/// Inject a permissions flag into an agent command string.
///
/// Inserts the flag after the executable token but before any existing arguments.
/// For commands like ` claude -- "$(cat PROMPT.md)"`, produces
/// ` claude --dangerously-skip-permissions -- "$(cat PROMPT.md)"`.
///
/// For non-POSIX wrapped commands like ` sh -c 'claude -- ...'`, the flag
/// is inserted inside the inner command.
pub fn inject_skip_permissions_flag(command: &str, flag: &str) -> String {
    // Handle the leading space (history prevention prefix)
    let trimmed = command.trim_start();
    let leading_spaces = &command[..command.len() - trimmed.len()];

    // Handle sh -c wrapper (non-POSIX shells)
    if trimmed.starts_with("sh -c '") && trimmed.ends_with('\'') {
        let inner = &trimmed[7..trimmed.len() - 1];
        let inner_unescaped = inner.replace("'\\''", "'");
        let injected = inject_flag_after_executable(&inner_unescaped, flag);
        let re_escaped = injected.replace('\'', "'\\''");
        return format!("{}sh -c '{}'", leading_spaces, re_escaped);
    }

    format!(
        "{}{}",
        leading_spaces,
        inject_flag_after_executable(trimmed, flag)
    )
}

/// Insert a flag after the first token (executable) in a simple command.
fn inject_flag_after_executable(command: &str, flag: &str) -> String {
    if let Some(space_idx) = command.find(' ') {
        let (exe, rest) = command.split_at(space_idx);
        format!("{} {}{}", exe, flag, rest)
    } else {
        format!("{} {}", command, flag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // --- prefixed tests ---

    #[test]
    fn test_prefixed() {
        assert_eq!(prefixed("wm-", "feature"), "wm-feature");
        assert_eq!(prefixed("", "feature"), "feature");
        assert_eq!(prefixed("prefix-", ""), "prefix-");
    }

    // --- is_posix_shell tests ---

    #[test]
    fn test_is_posix_shell_bash() {
        assert!(is_posix_shell("/bin/bash"));
        assert!(is_posix_shell("/usr/bin/bash"));
    }

    #[test]
    fn test_is_posix_shell_zsh() {
        assert!(is_posix_shell("/bin/zsh"));
        assert!(is_posix_shell("/usr/local/bin/zsh"));
    }

    #[test]
    fn test_is_posix_shell_sh() {
        assert!(is_posix_shell("/bin/sh"));
    }

    #[test]
    fn test_is_posix_shell_nushell() {
        assert!(!is_posix_shell("/opt/homebrew/bin/nu"));
        assert!(!is_posix_shell("/usr/bin/nu"));
    }

    #[test]
    fn test_is_posix_shell_fish() {
        assert!(!is_posix_shell("/usr/bin/fish"));
        assert!(!is_posix_shell("/opt/homebrew/bin/fish"));
    }

    // --- rewrite_agent_command tests for POSIX shells ---

    #[test]
    fn test_rewrite_claude_command_posix() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command(
            "claude",
            &prompt_file,
            &working_dir,
            Some("claude"),
            "/bin/zsh",
        );
        // POSIX shell: no wrapper, prefixed with space to prevent history
        assert_eq!(result, Some(" claude -- \"$(cat PROMPT.md)\"".to_string()));
    }

    #[test]
    fn test_rewrite_gemini_command_posix() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command(
            "gemini",
            &prompt_file,
            &working_dir,
            Some("gemini"),
            "/bin/bash",
        );
        assert_eq!(result, Some(" gemini -i \"$(cat PROMPT.md)\"".to_string()));
    }

    #[test]
    fn test_rewrite_opencode_command_posix() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command(
            "opencode",
            &prompt_file,
            &working_dir,
            Some("opencode"),
            "/bin/zsh",
        );
        assert_eq!(
            result,
            Some(" opencode --prompt \"$(cat PROMPT.md)\"".to_string())
        );
    }

    #[test]
    fn test_rewrite_command_with_args_posix() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command(
            "claude --verbose",
            &prompt_file,
            &working_dir,
            Some("claude"),
            "/bin/bash",
        );
        assert_eq!(
            result,
            Some(" claude --verbose -- \"$(cat PROMPT.md)\"".to_string())
        );
    }

    // --- rewrite_agent_command tests for non-POSIX shells ---

    #[test]
    fn test_rewrite_claude_command_nushell() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result = rewrite_agent_command(
            "claude",
            &prompt_file,
            &working_dir,
            Some("claude"),
            "/opt/homebrew/bin/nu",
        );
        // Non-POSIX shell: wrap in sh -c, prefixed with space
        assert_eq!(
            result,
            Some(" sh -c 'claude -- \"$(cat PROMPT.md)\"'".to_string())
        );
    }

    #[test]
    fn test_rewrite_mismatched_agent() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        // Command is for claude but agent is gemini
        let result = rewrite_agent_command(
            "claude",
            &prompt_file,
            &working_dir,
            Some("gemini"),
            "/bin/zsh",
        );
        assert_eq!(result, None);
    }

    #[test]
    fn test_rewrite_empty_command() {
        let prompt_file = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");

        let result =
            rewrite_agent_command("", &prompt_file, &working_dir, Some("claude"), "/bin/zsh");
        assert_eq!(result, None);
    }

    // --- escape_for_double_quotes tests ---

    #[test]
    fn test_escape_for_double_quotes_simple() {
        assert_eq!(escape_for_double_quotes("hello"), "hello");
        assert_eq!(escape_for_double_quotes("foo bar"), "foo bar");
    }

    #[test]
    fn test_escape_for_double_quotes_special_chars() {
        assert_eq!(escape_for_double_quotes("$HOME"), "\\$HOME");
        assert_eq!(escape_for_double_quotes("a\"b"), "a\\\"b");
        assert_eq!(escape_for_double_quotes("$(cmd)"), "\\$(cmd)");
        assert_eq!(escape_for_double_quotes("`cmd`"), "\\`cmd\\`");
    }

    #[test]
    fn test_escape_for_double_quotes_backslash() {
        assert_eq!(escape_for_double_quotes("a\\b"), "a\\\\b");
        assert_eq!(escape_for_double_quotes("\\$HOME"), "\\\\\\$HOME");
    }

    #[test]
    fn test_escape_for_double_quotes_combined() {
        // Test multiple special chars together
        assert_eq!(
            escape_for_double_quotes("echo \"$HOME\" `pwd`"),
            "echo \\\"\\$HOME\\\" \\`pwd\\`"
        );
    }

    // --- escape_for_sh_c_inner_single_quote tests ---

    #[test]
    fn test_escape_for_sh_c_inner_single_quote_simple() {
        assert_eq!(escape_for_sh_c_inner_single_quote("/bin/bash"), "/bin/bash");
    }

    #[test]
    fn test_escape_for_sh_c_inner_single_quote_with_single_quote() {
        // Shell path with single quote
        // Step 1: ' -> '\'' (single quote escaping)
        // Step 2: backslash in '\'' gets doubled for double-quote context -> '\\''
        assert_eq!(
            escape_for_sh_c_inner_single_quote("/bin/user's shell"),
            "/bin/user'\\\\''s shell"
        );
    }

    #[test]
    fn test_escape_for_sh_c_inner_single_quote_with_dollar() {
        // Dollar sign needs double-quote escaping
        assert_eq!(
            escape_for_sh_c_inner_single_quote("/path/$dir/shell"),
            "/path/\\$dir/shell"
        );
    }

    #[test]
    fn test_escape_for_sh_c_inner_single_quote_combined() {
        // Both single quote and dollar sign
        // Single quote becomes '\'' then backslash is doubled -> '\\''
        // Dollar sign becomes \$ (escaped for double quotes)
        assert_eq!(
            escape_for_sh_c_inner_single_quote("it's $HOME"),
            "it'\\\\''s \\$HOME"
        );
    }

    // --- wrap_for_non_posix_shell tests ---

    #[test]
    fn test_wrap_for_non_posix_shell_simple() {
        assert_eq!(wrap_for_non_posix_shell("echo hello"), "sh -c 'echo hello'");
    }

    #[test]
    fn test_wrap_for_non_posix_shell_with_single_quote() {
        assert_eq!(
            wrap_for_non_posix_shell("echo 'quoted'"),
            "sh -c 'echo '\\''quoted'\\'''"
        );
    }

    #[test]
    fn test_wrap_for_non_posix_shell_with_dollar() {
        // Dollar sign doesn't need escaping in single quotes
        assert_eq!(wrap_for_non_posix_shell("echo $HOME"), "sh -c 'echo $HOME'");
    }

    #[test]
    fn test_wrap_for_non_posix_shell_complex() {
        assert_eq!(
            wrap_for_non_posix_shell("claude -- \"$(cat PROMPT.md)\""),
            "sh -c 'claude -- \"$(cat PROMPT.md)\"'"
        );
    }

    // --- inject_skip_permissions_flag tests ---

    #[test]
    fn test_inject_skip_permissions_with_prompt() {
        let result = inject_skip_permissions_flag(
            " claude -- \"$(cat PROMPT.md)\"",
            "--dangerously-skip-permissions",
        );
        assert_eq!(
            result,
            " claude --dangerously-skip-permissions -- \"$(cat PROMPT.md)\""
        );
    }

    #[test]
    fn test_inject_skip_permissions_with_existing_args() {
        let result = inject_skip_permissions_flag(
            " claude --verbose -- \"$(cat PROMPT.md)\"",
            "--dangerously-skip-permissions",
        );
        assert_eq!(
            result,
            " claude --dangerously-skip-permissions --verbose -- \"$(cat PROMPT.md)\""
        );
    }

    #[test]
    fn test_inject_skip_permissions_bare_command() {
        let result = inject_skip_permissions_flag("claude", "--dangerously-skip-permissions");
        assert_eq!(result, "claude --dangerously-skip-permissions");
    }

    #[test]
    fn test_inject_skip_permissions_non_posix_shell() {
        let result = inject_skip_permissions_flag(
            " sh -c 'claude -- \"$(cat PROMPT.md)\"'",
            "--dangerously-skip-permissions",
        );
        assert_eq!(
            result,
            " sh -c 'claude --dangerously-skip-permissions -- \"$(cat PROMPT.md)\"'"
        );
    }

    // --- resolve_pane_command tests ---

    #[test]
    fn test_resolve_pane_command_none_when_no_command() {
        let result = resolve_pane_command(None, true, None, Path::new("/tmp"), None, "/bin/zsh");
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_pane_command_none_when_run_commands_false() {
        let result = resolve_pane_command(
            Some("echo hello"),
            false,
            None,
            Path::new("/tmp"),
            None,
            "/bin/zsh",
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_pane_command_returns_command_as_is() {
        let result =
            resolve_pane_command(Some("vim"), true, None, Path::new("/tmp"), None, "/bin/zsh");
        let resolved = result.unwrap();
        assert_eq!(resolved.command, "vim");
        assert!(!resolved.prompt_injected);
    }

    #[test]
    fn test_resolve_pane_command_agent_placeholder_with_agent() {
        let result = resolve_pane_command(
            Some("<agent>"),
            true,
            None,
            Path::new("/tmp"),
            Some("claude"),
            "/bin/zsh",
        );
        let resolved = result.unwrap();
        assert_eq!(resolved.command, "claude");
        assert!(!resolved.prompt_injected);
    }

    #[test]
    fn test_resolve_pane_command_agent_placeholder_without_agent() {
        let result = resolve_pane_command(
            Some("<agent>"),
            true,
            None,
            Path::new("/tmp"),
            None,
            "/bin/zsh",
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_resolve_pane_command_with_prompt_injection() {
        let prompt = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");
        let result = resolve_pane_command(
            Some("claude"),
            true,
            Some(&prompt),
            &working_dir,
            Some("claude"),
            "/bin/zsh",
        );
        let resolved = result.unwrap();
        assert!(resolved.prompt_injected);
        assert!(resolved.command.contains("PROMPT.md"));
    }

    #[test]
    fn test_resolve_pane_command_no_injection_for_mismatched_agent() {
        let prompt = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");
        let result = resolve_pane_command(
            Some("vim"),
            true,
            Some(&prompt),
            &working_dir,
            Some("claude"),
            "/bin/zsh",
        );
        let resolved = result.unwrap();
        assert!(!resolved.prompt_injected);
        assert_eq!(resolved.command, "vim");
    }

    #[test]
    fn test_resolve_pane_command_bare_agent_effective_agent_field() {
        let result = resolve_pane_command(
            Some("<agent>"),
            true,
            None,
            Path::new("/tmp"),
            Some("claude"),
            "/bin/zsh",
        );
        let resolved = result.unwrap();
        assert_eq!(resolved.command, "claude");
        assert_eq!(resolved.effective_agent.as_deref(), Some("claude"));
    }

    #[test]
    fn test_resolve_pane_command_regular_command_effective_agent_field() {
        let result = resolve_pane_command(
            Some("vim"),
            true,
            None,
            Path::new("/tmp"),
            Some("claude"),
            "/bin/zsh",
        );
        let resolved = result.unwrap();
        assert_eq!(resolved.command, "vim");
        // Regular commands still carry the window-level agent
        assert_eq!(resolved.effective_agent.as_deref(), Some("claude"));
    }

    // --- auto-detection of known agent commands ---

    #[test]
    fn test_resolve_pane_command_known_agent_auto_detected() {
        // "codex --flags" is a known agent, should auto-detect even when
        // the window-level agent is different
        let result = resolve_pane_command(
            Some("codex --yolo"),
            true,
            None,
            Path::new("/tmp"),
            Some("claude"), // window-level agent is claude
            "/bin/zsh",
        );
        let resolved = result.unwrap();
        assert_eq!(resolved.command, "codex --yolo");
        // effective_agent should be the command itself, not the window-level agent
        assert_eq!(resolved.effective_agent.as_deref(), Some("codex --yolo"));
    }

    #[test]
    fn test_resolve_pane_command_known_agent_prompt_injection() {
        let prompt = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");
        let result = resolve_pane_command(
            Some("codex"),
            true,
            Some(&prompt),
            &working_dir,
            Some("claude"), // window-level is claude, pane is codex
            "/bin/zsh",
        );
        let resolved = result.unwrap();
        assert!(resolved.prompt_injected);
        assert!(resolved.command.contains("PROMPT.md"));
        assert_eq!(resolved.effective_agent.as_deref(), Some("codex"));
    }

    #[test]
    fn test_resolve_pane_command_known_agent_no_window_agent() {
        // Known agent should work even without any window-level agent
        let prompt = PathBuf::from("/tmp/worktree/PROMPT.md");
        let working_dir = PathBuf::from("/tmp/worktree");
        let result = resolve_pane_command(
            Some("gemini"),
            true,
            Some(&prompt),
            &working_dir,
            None, // no window-level agent at all
            "/bin/zsh",
        );
        let resolved = result.unwrap();
        assert!(resolved.prompt_injected);
        // Should use gemini's profile (-i flag)
        assert!(resolved.command.contains("-i"));
        assert_eq!(resolved.effective_agent.as_deref(), Some("gemini"));
    }
}
