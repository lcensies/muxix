//! Agent profile system for extensible agent-specific behavior.
//!
//! This module defines the `AgentProfile` trait and built-in profiles for
//! known AI coding agents. Adding support for a new agent only requires
//! implementing this trait.

use std::path::Path;

/// Describes agent-specific behaviors for command rewriting and status handling.
pub trait AgentProfile: Send + Sync {
    /// Canonical name used for matching (e.g., "claude", "gemini").
    fn name(&self) -> &'static str;

    /// Whether this agent needs special handling for ! prefix (delay after !).
    ///
    /// Claude Code requires a small delay after sending `!` for it to register
    /// as a bash command.
    fn needs_bang_delay(&self) -> bool {
        false
    }

    /// Whether this agent needs auto-status when launched with a prompt file.
    ///
    /// Agents with hooks that would normally set status need auto-status as a
    /// workaround when launched with injected prompts. This is a workaround for
    /// Claude Code's broken UserPromptSubmit hook:
    /// <https://github.com/anthropics/claude-code/issues/17284>
    fn needs_auto_status(&self) -> bool {
        false
    }

    /// CLI flag to skip interactive permission prompts when running in a sandbox.
    ///
    /// Returns `None` for agents that don't support this, or a flag string
    /// like `--dangerously-skip-permissions` for agents that do.
    fn skip_permissions_flag(&self) -> Option<&'static str> {
        None
    }

    /// Format the prompt injection argument for this agent.
    ///
    /// Returns the CLI fragment to append (e.g., `-- "$(cat PROMPT.md)"`).
    fn prompt_argument(&self, prompt_path: &str) -> String {
        format!("-- \"$(cat {})\"", prompt_path)
    }
}

// === Built-in Profiles ===

pub struct ClaudeProfile;

impl AgentProfile for ClaudeProfile {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn needs_bang_delay(&self) -> bool {
        true
    }

    fn needs_auto_status(&self) -> bool {
        true
    }

    fn skip_permissions_flag(&self) -> Option<&'static str> {
        Some("--dangerously-skip-permissions")
    }
}

pub struct GeminiProfile;

impl AgentProfile for GeminiProfile {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn skip_permissions_flag(&self) -> Option<&'static str> {
        Some("--yolo")
    }

    fn prompt_argument(&self, prompt_path: &str) -> String {
        format!("-i \"$(cat {})\"", prompt_path)
    }
}

pub struct OpenCodeProfile;

impl AgentProfile for OpenCodeProfile {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn needs_auto_status(&self) -> bool {
        true
    }

    fn prompt_argument(&self, prompt_path: &str) -> String {
        format!("--prompt \"$(cat {})\"", prompt_path)
    }
}

pub struct CodexProfile;

impl AgentProfile for CodexProfile {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn skip_permissions_flag(&self) -> Option<&'static str> {
        Some("--yolo")
    }
}

pub struct DefaultProfile;

impl AgentProfile for DefaultProfile {
    fn name(&self) -> &'static str {
        "default"
    }
}

// === Registry ===

static PROFILES: &[&dyn AgentProfile] = &[
    &ClaudeProfile,
    &GeminiProfile,
    &OpenCodeProfile,
    &CodexProfile,
];

/// Check if a command matches a known agent profile.
///
/// Returns true for commands whose executable stem matches a built-in agent
/// (claude, gemini, codex, opencode). Used for auto-detecting agent panes
/// without requiring the `<agent>` placeholder.
pub fn is_known_agent(command: &str) -> bool {
    let stem = extract_executable_stem(command);
    PROFILES.iter().any(|p| p.name() == stem)
}

/// Resolve an agent command to its profile.
///
/// Returns `DefaultProfile` if no specific profile matches.
pub fn resolve_profile(agent_command: Option<&str>) -> &'static dyn AgentProfile {
    let Some(cmd) = agent_command else {
        return &DefaultProfile;
    };

    let stem = extract_executable_stem(cmd);

    PROFILES
        .iter()
        .find(|p| p.name() == stem)
        .copied()
        .unwrap_or(&DefaultProfile)
}

/// Extract the executable stem from a command string.
///
/// Examples:
/// - "claude --verbose" -> "claude"
/// - "/usr/bin/gemini" -> "gemini"
fn extract_executable_stem(command: &str) -> String {
    let (token, _) = crate::config::split_first_token(command).unwrap_or((command, ""));

    // Resolve the path to handle symlinks and aliases
    let resolved =
        crate::config::resolve_executable_path(token).unwrap_or_else(|| token.to_string());

    // Extract stem from the resolved path
    Path::new(&resolved)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Profile behavior tests ===

    #[test]
    fn test_claude_profile() {
        let profile = ClaudeProfile;
        assert_eq!(profile.name(), "claude");
        assert!(profile.needs_bang_delay());
        assert!(profile.needs_auto_status());
        assert_eq!(
            profile.prompt_argument("PROMPT.md"),
            "-- \"$(cat PROMPT.md)\""
        );
        assert_eq!(
            profile.skip_permissions_flag(),
            Some("--dangerously-skip-permissions")
        );
    }

    #[test]
    fn test_gemini_profile() {
        let profile = GeminiProfile;
        assert_eq!(profile.name(), "gemini");
        assert!(!profile.needs_bang_delay());
        assert!(!profile.needs_auto_status());
        assert_eq!(
            profile.prompt_argument("PROMPT.md"),
            "-i \"$(cat PROMPT.md)\""
        );
        assert_eq!(profile.skip_permissions_flag(), Some("--yolo"));
    }

    #[test]
    fn test_opencode_profile() {
        let profile = OpenCodeProfile;
        assert_eq!(profile.name(), "opencode");
        assert!(!profile.needs_bang_delay());
        assert!(profile.needs_auto_status());
        assert_eq!(
            profile.prompt_argument("PROMPT.md"),
            "--prompt \"$(cat PROMPT.md)\""
        );
    }

    #[test]
    fn test_codex_profile() {
        let profile = CodexProfile;
        assert_eq!(profile.name(), "codex");
        assert!(!profile.needs_bang_delay());
        assert!(!profile.needs_auto_status());
        assert_eq!(
            profile.prompt_argument("PROMPT.md"),
            "-- \"$(cat PROMPT.md)\""
        );
        assert_eq!(profile.skip_permissions_flag(), Some("--yolo"));
    }

    #[test]
    fn test_default_profile() {
        let profile = DefaultProfile;
        assert_eq!(profile.name(), "default");
        assert!(!profile.needs_bang_delay());
        assert!(!profile.needs_auto_status());
        assert_eq!(
            profile.prompt_argument("PROMPT.md"),
            "-- \"$(cat PROMPT.md)\""
        );
    }

    // === resolve_profile tests ===

    #[test]
    fn test_resolve_profile_none() {
        let profile = resolve_profile(None);
        assert_eq!(profile.name(), "default");
    }

    #[test]
    fn test_resolve_profile_claude() {
        let profile = resolve_profile(Some("claude"));
        assert_eq!(profile.name(), "claude");
    }

    #[test]
    fn test_resolve_profile_claude_with_args() {
        let profile = resolve_profile(Some("claude --verbose"));
        assert_eq!(profile.name(), "claude");
    }

    #[test]
    fn test_resolve_profile_gemini() {
        let profile = resolve_profile(Some("gemini"));
        assert_eq!(profile.name(), "gemini");
    }

    #[test]
    fn test_resolve_profile_opencode() {
        let profile = resolve_profile(Some("opencode"));
        assert_eq!(profile.name(), "opencode");
    }

    #[test]
    fn test_resolve_profile_codex() {
        let profile = resolve_profile(Some("codex"));
        assert_eq!(profile.name(), "codex");
    }

    #[test]
    fn test_resolve_profile_unknown() {
        let profile = resolve_profile(Some("unknown-agent"));
        assert_eq!(profile.name(), "default");
    }

    // === is_known_agent tests ===

    #[test]
    fn test_is_known_agent_bare_names() {
        assert!(is_known_agent("claude"));
        assert!(is_known_agent("gemini"));
        assert!(is_known_agent("codex"));
        assert!(is_known_agent("opencode"));
    }

    #[test]
    fn test_is_known_agent_with_args() {
        assert!(is_known_agent("claude --dangerously-skip-permissions"));
        assert!(is_known_agent("codex --yolo"));
        assert!(is_known_agent("gemini -i foo"));
    }

    #[test]
    fn test_is_known_agent_unknown() {
        assert!(!is_known_agent("vim"));
        assert!(!is_known_agent("npm run dev"));
        assert!(!is_known_agent("clear"));
        assert!(!is_known_agent("unknown-agent"));
    }
}
