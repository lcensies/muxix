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

/// Escape a string for safe inclusion in shell commands using single quotes.
///
/// This wraps the string in single quotes and escapes any embedded single quotes
/// using the `'\''` pattern (end quote, escaped quote, start quote).
pub fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', r#"'\''"#))
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

    // --- shell_escape tests ---

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "'hello'");
        assert_eq!(shell_escape("foo bar"), "'foo bar'");
    }

    #[test]
    fn test_shell_escape_with_single_quote() {
        // Single quote becomes '\'' (end quote, escaped quote, start quote)
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
        assert_eq!(shell_escape("don't"), "'don'\\''t'");
    }

    #[test]
    fn test_shell_escape_with_special_chars() {
        assert_eq!(shell_escape("foo$bar"), "'foo$bar'");
        assert_eq!(shell_escape("a*b?c"), "'a*b?c'");
        assert_eq!(shell_escape("hello\nworld"), "'hello\nworld'");
    }

    #[test]
    fn test_shell_escape_empty() {
        assert_eq!(shell_escape(""), "''");
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
}
