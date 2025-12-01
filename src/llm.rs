use anyhow::{Context, Result, anyhow};
use std::io::Write;
use std::process::{Command, Stdio};

const SYSTEM_PROMPT: &str = r#"Generate a short, valid git branch name (kebab-case) based on the user's input.
Output ONLY the branch name.

User Input:
"#;

pub fn generate_branch_name(prompt: &str, model: Option<&str>) -> Result<String> {
    let full_prompt = format!("{}{}", SYSTEM_PROMPT, prompt);

    let mut cmd = Command::new("llm");
    if let Some(m) = model {
        cmd.args(["-m", m]);
    }

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
        return Err(anyhow!("llm command failed: {}", stderr));
    }

    let raw = String::from_utf8(output.stdout)?;
    let branch_name = sanitize_branch_name(raw.trim());

    if branch_name.is_empty() {
        return Err(anyhow!("LLM returned empty branch name"));
    }

    Ok(branch_name)
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
}
