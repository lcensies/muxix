//! Direct-exec (argv) launch for agent panes.
//!
//! tmux's `new-window`, `new-session`, `split-window`, `respawn-window` and
//! `respawn-pane` accept a command given as *multiple arguments* and exec it
//! directly, without `sh -c`. Building an argv here — instead of a shell
//! command line — removes every shell layer from the launch path:
//!
//! * the prompt travels as one argument of arbitrary bytes, so nothing has to
//!   quote it, escape it, or parse it back apart;
//! * a missing or unreadable prompt file is a hard error instead of an
//!   `$(cat ...)` that silently expands to the empty string;
//! * flag injection is `Vec::insert` on real arguments rather than string
//!   surgery on a finished command line;
//! * no `sh -c` wrapping is needed for non-POSIX login shells, because no
//!   login shell is involved.

use anyhow::{Result, anyhow};

use crate::agent::profile::{AgentProfile, executable_index};

/// Largest prompt passed as an argv element.
///
/// `execve` caps a *single* argument at `MAX_ARG_STRLEN` (128 KiB on Linux)
/// and the whole argv+env block at ~2 MiB; macOS caps the block at 256 KiB.
/// Staying under the smallest of those keeps behaviour identical across
/// platforms — larger prompts fall back to the file-reading command form.
pub const MAX_ARGV_PROMPT_BYTES: usize = 96 * 1024;

/// Whether a prompt is small enough to pass directly in argv.
pub fn prompt_fits_argv(prompt: &str) -> bool {
    prompt.len() <= MAX_ARGV_PROMPT_BYTES
}

/// Build the argv for launching an agent directly, with no shell involved.
///
/// `command` is the user's configured agent command (possibly with an `env`
/// wrapper and arguments); `prompt` is the prompt *contents*, not a path.
/// `extra_flags` are injected after the executable, each skipped if already
/// present. Because the prompt is appended last and never scanned, a prompt
/// containing a flag's text cannot suppress that flag's injection.
pub fn build_agent_argv(
    command: &str,
    profile: &dyn AgentProfile,
    prompt: Option<&str>,
    extra_flags: &[&str],
) -> Result<Vec<String>> {
    let mut argv = shlex::split(command)
        .ok_or_else(|| anyhow!("agent command is not valid shell syntax: {command}"))?;
    if argv.is_empty() {
        return Err(anyhow!("agent command is empty"));
    }

    let mut insert_at = executable_index(&argv) + 1;

    // The default subcommand (e.g. `kiro-cli` -> `kiro-cli chat`) is a
    // positional and must precede any injected flags.
    if let Some(subcmd) = profile.default_subcommand()
        && needs_subcommand(argv.get(insert_at).map(String::as_str), subcmd)
    {
        argv.insert(insert_at, subcmd.to_string());
        insert_at += 1;
    }

    let mut injected: Vec<String> = Vec::new();
    for flag in extra_flags {
        let tokens = split_flag(flag);
        if tokens.is_empty()
            || contains_tokens(&argv, &tokens)
            || contains_tokens(&injected, &tokens)
        {
            continue;
        }
        injected.extend(tokens);
    }
    argv.splice(insert_at..insert_at, injected);

    if let Some(prompt) = prompt {
        if let Some(flag) = profile.prompt_flag() {
            argv.extend(split_flag(flag));
        }
        argv.push(prompt.to_string());
    }

    Ok(argv)
}

/// Split a flag that may carry its own argument (e.g. `--agent auto-approve`).
fn split_flag(flag: &str) -> Vec<String> {
    shlex::split(flag).unwrap_or_else(|| flag.split_whitespace().map(str::to_string).collect())
}

/// Whether `needle` appears as a contiguous run of arguments in `haystack`.
fn contains_tokens(haystack: &[String], needle: &[String]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Whether the default subcommand still needs inserting.
///
/// Flags are not subcommands, so the default is still inserted before them.
fn needs_subcommand(first_arg: Option<&str>, subcmd: &str) -> bool {
    match first_arg {
        None => true,
        Some(a) if a == subcmd => false,
        Some(a) if a.starts_with('-') => true,
        Some(_) => false,
    }
}

/// Open a pane running `argv` under `placement`.
///
/// Shared by the ADE session view and native takeover so both land under the
/// same policy, and so a backend without direct exec degrades to a quoted
/// command line in one place rather than two.
///
/// Placement is advisory: a backend that cannot honour it falls back to a plain
/// split rather than refusing to open the session at all.
pub fn open_argv_pane(
    mux: &dyn crate::multiplexer::Multiplexer,
    argv: &[String],
    cwd: &std::path::Path,
    placement: crate::config::AdeViewPlacement,
    window_name: Option<&str>,
) -> Result<String> {
    use crate::config::AdeViewPlacement as P;

    match placement {
        P::Split => split_beside_current(mux, argv, cwd),
        P::Window => match current_session_window(mux, argv, cwd, window_name) {
            Ok(pane) => Ok(pane),
            // No session to put a window in: the session view matters more than
            // where it sits.
            Err(_) => split_beside_current(mux, argv, cwd),
        },
        P::Project => match project_window(mux, argv, cwd, window_name) {
            Ok(pane) => Ok(pane),
            Err(_) => match current_session_window(mux, argv, cwd, window_name) {
                Ok(pane) => Ok(pane),
                Err(_) => split_beside_current(mux, argv, cwd),
            },
        },
    }
}

/// One window per agent inside the tmux session named for the project owning
/// `cwd`, creating that session when it does not exist yet.
///
/// This is the shape `muxix open` gives a project (`projects::start`), so an ADE
/// session lands beside the project's own windows instead of inside whichever
/// project the user happened to be looking at.
fn project_window(
    mux: &dyn crate::multiplexer::Multiplexer,
    argv: &[String],
    cwd: &std::path::Path,
    window_name: Option<&str>,
) -> Result<String> {
    let registry = crate::projects::registry::Registry::load()?;
    let entry = cwd
        .to_str()
        .and_then(|p| registry.find(p))
        .ok_or_else(|| anyhow!("{} belongs to no registered project", cwd.display()))?
        .clone();

    if !mux.session_exists(&entry.name)? {
        mux.create_session(crate::multiplexer::CreateSessionParams {
            prefix: "",
            name: &entry.name,
            cwd: &entry.root,
            initial_window_name: None,
        })?;
    }

    let pane = mux.create_window_in_session(crate::multiplexer::CreateWindowInSessionParams {
        session_name: &entry.name,
        name: window_name,
        cwd,
    })?;
    run_in_pane(mux, &pane, argv)?;
    // Best effort: the window exists either way, and a failed switch must not
    // read as a failed open.
    let _ = mux.switch_to_session("", &entry.name);
    Ok(pane)
}

/// One window per agent in the session that is already current.
fn current_session_window(
    mux: &dyn crate::multiplexer::Multiplexer,
    argv: &[String],
    cwd: &std::path::Path,
    window_name: Option<&str>,
) -> Result<String> {
    let session = mux
        .current_session()
        .ok_or_else(|| anyhow!("not inside a session, so there is no window to add"))?;
    let pane = mux.create_window_in_session(crate::multiplexer::CreateWindowInSessionParams {
        session_name: &session,
        name: window_name,
        cwd,
    })?;
    run_in_pane(mux, &pane, argv)?;
    Ok(pane)
}

/// Start `argv` in an already-created pane.
///
/// There is no create-window-with-argv primitive, so the window primitives take
/// the same quoted-line path `projects::start` uses for layout commands.
fn run_in_pane(
    mux: &dyn crate::multiplexer::Multiplexer,
    pane: &str,
    argv: &[String],
) -> Result<()> {
    let line = argv
        .iter()
        .map(|a| crate::shell::shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    mux.send_keys(pane, &line)
}

fn split_beside_current(
    mux: &dyn crate::multiplexer::Multiplexer,
    argv: &[String],
    cwd: &std::path::Path,
) -> Result<String> {
    let target = mux
        .current_pane_id()
        .or_else(|| mux.active_pane_id())
        .ok_or_else(|| anyhow!("no current pane to open beside"))?;

    if mux.supports_direct_exec() {
        return mux.split_pane_argv(
            &target,
            &crate::multiplexer::SplitDirection::Horizontal,
            cwd,
            None,
            Some(50),
            argv,
        );
    }

    let line = argv
        .iter()
        .map(|a| crate::shell::shell_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    mux.split_pane(
        &target,
        &crate::multiplexer::SplitDirection::Horizontal,
        cwd,
        None,
        Some(50),
        Some(&line),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::profile::resolve_profile;

    fn argv(command: &str, prompt: Option<&str>, flags: &[&str]) -> Vec<String> {
        let profile = resolve_profile(Some(command));
        build_agent_argv(command, profile, prompt, flags).unwrap()
    }

    #[test]
    fn prompt_is_a_single_argument() {
        assert_eq!(
            argv("claude", Some("hello world"), &[]),
            vec!["claude", "--", "hello world"]
        );
    }

    /// The whole point: prompt content that would wreck a shell command line
    /// is inert as an argv element.
    #[test]
    fn prompt_with_shell_metacharacters_is_untouched() {
        let nasty = "rm -rf / ; $(whoami) `id` \"quoted\" 'single' \\ \n newline";
        let out = argv("claude", Some(nasty), &[]);
        assert_eq!(out.last().unwrap(), nasty);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn flags_are_injected_after_the_executable() {
        assert_eq!(
            argv(
                "claude --verbose",
                Some("p"),
                &["--dangerously-skip-permissions"]
            ),
            vec![
                "claude",
                "--dangerously-skip-permissions",
                "--verbose",
                "--",
                "p"
            ]
        );
    }

    #[test]
    fn flags_are_not_injected_twice() {
        assert_eq!(
            argv(
                "claude --dangerously-skip-permissions",
                None,
                &["--dangerously-skip-permissions"]
            ),
            vec!["claude", "--dangerously-skip-permissions"]
        );
    }

    /// Regression: a prompt containing the flag's text used to suppress
    /// injection, because presence was checked by scanning the whole line.
    #[test]
    fn prompt_containing_flag_text_does_not_suppress_injection() {
        let out = argv(
            "claude",
            Some("please do not pass --dangerously-skip-permissions"),
            &["--dangerously-skip-permissions"],
        );
        assert_eq!(out[1], "--dangerously-skip-permissions");
        assert_eq!(
            out.last().unwrap(),
            "please do not pass --dangerously-skip-permissions"
        );
    }

    #[test]
    fn env_wrapper_keeps_flags_on_the_agent_not_on_env() {
        assert_eq!(
            argv(
                "env -u CLAUDE_CODE_USE_BEDROCK claude",
                Some("p"),
                &["--dangerously-skip-permissions"]
            ),
            vec![
                "env",
                "-u",
                "CLAUDE_CODE_USE_BEDROCK",
                "claude",
                "--dangerously-skip-permissions",
                "--",
                "p"
            ]
        );
    }

    #[test]
    fn var_assignment_wrapper_is_handled() {
        assert_eq!(
            argv("FOO=bar claude", Some("p"), &[]),
            vec!["FOO=bar", "claude", "--", "p"]
        );
    }

    #[test]
    fn quoted_config_arguments_survive_as_one_argument() {
        let out = argv(r#"codex --config model_reasoning_effort="low""#, None, &[]);
        assert_eq!(out, vec!["codex", "--config", "model_reasoning_effort=low"]);
    }

    #[test]
    fn multi_token_flag_is_injected_whole() {
        assert_eq!(
            argv("vibe", None, &["--agent auto-approve"]),
            vec!["vibe", "--agent", "auto-approve"]
        );
    }

    #[test]
    fn default_subcommand_precedes_injected_flags() {
        assert_eq!(
            argv("kiro-cli", Some("p"), &["--resume"]),
            vec!["kiro-cli", "chat", "--resume", "p"]
        );
    }

    #[test]
    fn default_subcommand_not_duplicated() {
        let out = argv("kiro-cli chat", None, &[]);
        assert_eq!(out, vec!["kiro-cli", "chat"]);
    }

    #[test]
    fn profile_without_prompt_flag_passes_prompt_positionally() {
        assert_eq!(argv("pi", Some("p"), &[]), vec!["pi", "p"]);
    }

    #[test]
    fn unbalanced_quotes_are_an_error_not_a_silent_mangle() {
        let profile = resolve_profile(Some("claude"));
        assert!(build_agent_argv("claude --flag 'unterminated", profile, None, &[]).is_err());
    }

    #[test]
    fn oversized_prompt_is_rejected_by_the_size_check() {
        assert!(prompt_fits_argv("small"));
        assert!(!prompt_fits_argv(&"x".repeat(MAX_ARGV_PROMPT_BYTES + 1)));
    }
}
