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

    /// Subcommand to insert after the executable when launching.
    ///
    /// For agents like kiro-cli where the bare executable shows a menu
    /// rather than starting chat, this returns the subcommand needed
    /// (e.g., `"chat"` so that `kiro-cli` becomes `kiro-cli chat`).
    ///
    /// Skipped if the user already includes it in their config
    /// (e.g., `agent: "kiro-cli chat"`).
    fn default_subcommand(&self) -> Option<&'static str> {
        None
    }

    /// Default command for auto-naming branches with this agent's CLI.
    ///
    /// Returns a fast/cheap command string suitable for branch name generation,
    /// or `None` if this profile has no known auto-name command.
    fn auto_name_command(&self) -> Option<&'static str> {
        None
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

    fn auto_name_command(&self) -> Option<&'static str> {
        Some("claude --model haiku -p")
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

    fn auto_name_command(&self) -> Option<&'static str> {
        Some("gemini -m gemini-2.5-flash-lite -p")
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

    fn auto_name_command(&self) -> Option<&'static str> {
        Some("opencode run")
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

    fn auto_name_command(&self) -> Option<&'static str> {
        Some(r#"codex exec --config model_reasoning_effort="low" -m gpt-5.1-codex-mini"#)
    }
}

pub struct KiroProfile;

impl AgentProfile for KiroProfile {
    fn name(&self) -> &'static str {
        "kiro-cli"
    }

    fn default_subcommand(&self) -> Option<&'static str> {
        Some("chat")
    }

    fn prompt_argument(&self, prompt_path: &str) -> String {
        format!("\"$(cat {})\"", prompt_path)
    }

    fn auto_name_command(&self) -> Option<&'static str> {
        Some("kiro-cli chat --no-interactive")
    }
}

pub struct VibeProfile;

impl AgentProfile for VibeProfile {
    fn name(&self) -> &'static str {
        "vibe"
    }

    fn skip_permissions_flag(&self) -> Option<&'static str> {
        Some("--agent auto-approve")
    }

    fn prompt_argument(&self, prompt_path: &str) -> String {
        format!("\"$(cat {})\"", prompt_path)
    }
}

pub struct PiProfile;

impl AgentProfile for PiProfile {
    fn name(&self) -> &'static str {
        "pi"
    }

    fn needs_auto_status(&self) -> bool {
        true
    }

    fn prompt_argument(&self, prompt_path: &str) -> String {
        format!("-p \"$(cat {})\"", prompt_path)
    }

    fn auto_name_command(&self) -> Option<&'static str> {
        Some("pi -p")
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
    &PiProfile,
    &KiroProfile,
    &VibeProfile,
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
        assert_eq!(profile.auto_name_command(), Some("claude --model haiku -p"));
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
        assert_eq!(
            profile.auto_name_command(),
            Some("gemini -m gemini-2.5-flash-lite -p")
        );
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
        assert_eq!(profile.auto_name_command(), Some("opencode run"));
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
        assert_eq!(
            profile.auto_name_command(),
            Some(r#"codex exec --config model_reasoning_effort="low" -m gpt-5.1-codex-mini"#)
        );
    }

    #[test]
    fn test_kiro_profile() {
        let profile = KiroProfile;
        assert_eq!(profile.name(), "kiro-cli");
        assert!(!profile.needs_bang_delay());
        assert!(!profile.needs_auto_status());
        assert_eq!(profile.default_subcommand(), Some("chat"));
        assert_eq!(profile.prompt_argument("PROMPT.md"), "\"$(cat PROMPT.md)\"");
        assert_eq!(profile.skip_permissions_flag(), None);
        assert_eq!(
            profile.auto_name_command(),
            Some("kiro-cli chat --no-interactive")
        );
    }

    #[test]
    fn test_vibe_profile() {
        let profile = VibeProfile;
        assert_eq!(profile.name(), "vibe");
        assert!(!profile.needs_bang_delay());
        assert!(!profile.needs_auto_status());
        assert_eq!(profile.prompt_argument("PROMPT.md"), "\"$(cat PROMPT.md)\"");
        assert_eq!(
            profile.skip_permissions_flag(),
            Some("--agent auto-approve")
        );
        assert_eq!(profile.auto_name_command(), None);
    }

    #[test]
    fn test_pi_profile() {
        let profile = PiProfile;
        assert_eq!(profile.name(), "pi");
        assert!(!profile.needs_bang_delay());
        assert!(profile.needs_auto_status());
        assert_eq!(
            profile.prompt_argument("PROMPT.md"),
            "-p \"$(cat PROMPT.md)\""
        );
        assert_eq!(profile.skip_permissions_flag(), None);
        assert_eq!(profile.auto_name_command(), Some("pi -p"));
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
        assert_eq!(profile.auto_name_command(), None);
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
    fn test_resolve_profile_pi() {
        let profile = resolve_profile(Some("pi"));
        assert_eq!(profile.name(), "pi");
    }

    #[test]
    fn test_resolve_profile_codex() {
        let profile = resolve_profile(Some("codex"));
        assert_eq!(profile.name(), "codex");
    }

    #[test]
    fn test_resolve_profile_kiro() {
        let profile = resolve_profile(Some("kiro-cli"));
        assert_eq!(profile.name(), "kiro-cli");
    }

    #[test]
    fn test_resolve_profile_kiro_with_subcommand() {
        let profile = resolve_profile(Some("kiro-cli chat"));
        assert_eq!(profile.name(), "kiro-cli");
    }

    #[test]
    fn test_resolve_profile_vibe() {
        let profile = resolve_profile(Some("vibe"));
        assert_eq!(profile.name(), "vibe");
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
        assert!(is_known_agent("pi"));
        assert!(is_known_agent("kiro-cli"));
        assert!(is_known_agent("vibe"));
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
