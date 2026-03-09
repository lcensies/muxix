use anyhow::{Context, Result, anyhow};
use std::io::Write;
use std::process::{Command, Stdio};

const DEFAULT_SYSTEM_PROMPT: &str = r#"Generate a short, valid git branch name (kebab-case) based on the user's input.
Output ONLY the branch name."#;

pub fn generate_branch_name(
    prompt: &str,
    model: Option<&str>,
    system_prompt: Option<&str>,
    command: Option<&str>,
) -> Result<String> {
    let system = system_prompt.unwrap_or(DEFAULT_SYSTEM_PROMPT);
    let full_prompt = format!("{}\n\nUser Input:\n{}", system, prompt);

    tracing::info!(
        user_prompt = prompt,
        system_prompt = system,
        model = model.unwrap_or("default"),
        command = command.unwrap_or("llm"),
        "generating branch name"
    );
    tracing::info!(full_prompt = full_prompt, "full prompt sent to generator");

    let raw = run_generator_command(command, model, &full_prompt)?;
    tracing::info!(raw_output = raw.trim(), "raw output from generator");

    let branch_name = sanitize_branch_name(raw.trim());
    tracing::info!(branch_name = branch_name, "sanitized branch name");

    if branch_name.is_empty() {
        tracing::error!(
            raw_output = raw.trim(),
            "generator returned empty branch name after sanitization"
        );
        return Err(anyhow!("LLM returned empty branch name"));
    }

    Ok(branch_name)
}

fn run_generator_command(
    command: Option<&str>,
    model: Option<&str>,
    full_prompt: &str,
) -> Result<String> {
    match command.map(str::trim).filter(|s| !s.is_empty()) {
        Some("llm") | None => run_llm_command(model, full_prompt),
        Some(cmdline) => run_custom_command(cmdline, full_prompt),
    }
}

fn run_custom_command(cmdline: &str, full_prompt: &str) -> Result<String> {
    let parts = shlex::split(cmdline).ok_or_else(|| {
        anyhow!(
            "Failed to parse auto_name.command: mismatched quotes in '{}'",
            cmdline
        )
    })?;

    if parts.is_empty() {
        anyhow::bail!("auto_name.command is empty");
    }

    let program = &parts[0];
    let fixed_args = &parts[1..];

    tracing::info!(
        program = program.as_str(),
        args = ?fixed_args,
        "running custom generator command"
    );

    let mut child = Command::new(program)
        .args(fixed_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("Failed to execute custom command '{}'", program))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(full_prompt.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let msg = if stderr.trim().is_empty() {
            String::from_utf8_lossy(&output.stdout)
        } else {
            stderr
        };
        tracing::error!(
            program = program.as_str(),
            exit_code = output.status.code().unwrap_or(1),
            stderr = msg.trim(),
            "custom generator command failed"
        );
        anyhow::bail!(
            "Custom command '{}' failed (exit code {}):\n{}",
            program,
            output.status.code().unwrap_or(1),
            msg.trim()
        );
    }

    Ok(String::from_utf8(output.stdout)?)
}

fn run_llm_command(model: Option<&str>, full_prompt: &str) -> Result<String> {
    let mut cmd = Command::new("llm");
    if let Some(m) = model {
        cmd.args(["-m", m]);
    }

    tracing::info!(model = model.unwrap_or("default"), "running llm command");

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to run 'llm' command. Is it installed? (pipx install llm)")?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(full_prompt.as_bytes())?;
    }

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(stderr = %stderr, "llm command failed");
        return Err(anyhow!("llm command failed: {}", stderr));
    }

    Ok(String::from_utf8(output.stdout)?)
}

fn sanitize_branch_name(raw: &str) -> String {
    // Remove markdown code blocks if present
    let cleaned = raw
        .trim_matches('`')
        .trim()
        .lines()
        .next()
        .unwrap_or("")
        .trim();

    // Use slug to ensure valid format
    slug::slugify(cleaned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_branch_name_simple() {
        assert_eq!(sanitize_branch_name("add-user-auth"), "add-user-auth");
    }

    #[test]
    fn sanitize_branch_name_with_backticks() {
        assert_eq!(sanitize_branch_name("`add-user-auth`"), "add-user-auth");
    }

    #[test]
    fn sanitize_branch_name_with_triple_backticks() {
        assert_eq!(
            sanitize_branch_name("```\nadd-user-auth\n```"),
            "add-user-auth"
        );
    }

    #[test]
    fn sanitize_branch_name_multiline() {
        assert_eq!(
            sanitize_branch_name("add-user-auth\nsome explanation"),
            "add-user-auth"
        );
    }

    #[test]
    fn sanitize_branch_name_with_spaces() {
        assert_eq!(sanitize_branch_name("add user auth"), "add-user-auth");
    }

    #[test]
    fn sanitize_branch_name_with_special_chars() {
        assert_eq!(sanitize_branch_name("Add User Auth!"), "add-user-auth");
    }

    #[test]
    fn sanitize_branch_name_empty() {
        assert_eq!(sanitize_branch_name(""), "");
    }

    #[test]
    fn sanitize_branch_name_whitespace_only() {
        assert_eq!(sanitize_branch_name("   "), "");
    }

    #[test]
    fn run_generator_dispatches_to_custom_command() {
        // When command is set, it should attempt to run the custom command
        // (will fail because "nonexistent-test-cmd" doesn't exist, but proves dispatch)
        let result = run_generator_command(Some("nonexistent-test-cmd"), Some("model"), "prompt");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("nonexistent-test-cmd"),
            "Error should mention the custom command: {}",
            err
        );
    }

    #[test]
    fn run_generator_routes_bare_llm_to_llm_command() {
        // "llm" as the command string should route to run_llm_command (stdin-based path),
        // not run_custom_command. Both will fail if llm isn't installed, but the error
        // message differs: run_custom_command appends the prompt as an arg, while
        // run_llm_command uses stdin and mentions "llm" in its error.
        let result = run_generator_command(Some("llm"), Some("model"), "prompt");
        // Either llm is installed (ok) or it fails with the llm-specific error.
        // The key assertion: it must NOT treat "llm" as a custom command (which would
        // call `llm prompt` with prompt as an argument, producing a different error).
        if let Err(e) = result {
            let err = e.to_string();
            // run_llm_command produces "Failed to run 'llm' command" or "llm command failed"
            assert!(err.contains("llm"), "Error should mention llm: {}", err);
            // run_custom_command would produce "Failed to execute custom command"
            assert!(
                !err.contains("Failed to execute custom command"),
                "Should not be routed to run_custom_command: {}",
                err
            );
        }
    }

    #[test]
    fn custom_command_rejects_mismatched_quotes() {
        let result = run_custom_command("claude --sys \"unclosed", "prompt");
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("mismatched quotes"),
            "Should report mismatched quotes: {}",
            err
        );
    }
}
