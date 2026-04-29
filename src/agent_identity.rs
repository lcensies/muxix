//! Agent identity classification.
//!
//! Classifies an agent pane by combining tmux's `pane_current_command` with
//! the pane title. Some agents report a version string (Claude Code: "2.1.118"),
//! a truncated binary name (Codex: "codex-aarch64-a"), or run as a generic
//! interpreter (Gemini, Pi, Vibe). Stem-based profile resolution alone misses
//! these, so the result of `classify_agent_kind` is cached on `AgentState`
//! once it becomes non-None and reused by the sidebar render path.
//!
//! The returned string is the canonical profile name (e.g. "claude",
//! "kiro-cli") so the sidebar can look up the existing `AgentProfile`.

use std::path::Path;

const GENERIC_INTERPRETERS: &[&str] = &["node", "python", "python3", "bun", "deno"];

/// Classify an agent pane using its foreground command and pane title.
///
/// Returns the canonical profile name (e.g. "claude") or `None` if no rule
/// matches. Callers cache the first non-None result to avoid re-classifying
/// on every tick.
pub fn classify_agent_kind(command: Option<&str>, pane_title: Option<&str>) -> Option<String> {
    let raw = command.unwrap_or("").trim();
    let stem = command_stem(raw);

    if let Some(kind) = classify_by_command(raw, &stem) {
        return Some(kind.to_string());
    }

    if is_generic_interpreter(&stem)
        && let Some(kind) = classify_by_title(pane_title.unwrap_or(""))
    {
        return Some(kind.to_string());
    }

    None
}

fn classify_by_command(raw: &str, stem: &str) -> Option<&'static str> {
    if stem.is_empty() {
        return None;
    }

    if is_version_string(stem) || is_version_string(raw) {
        return Some("claude");
    }

    if stem == "codex" || stem.starts_with("codex-") {
        return Some("codex");
    }

    match stem {
        "claude" => Some("claude"),
        "opencode" => Some("opencode"),
        "kiro-cli" => Some("kiro-cli"),
        "copilot" => Some("copilot"),
        "gemini" => Some("gemini"),
        "pi" => Some("pi"),
        "vibe" => Some("vibe"),
        _ => None,
    }
}

fn classify_by_title(title: &str) -> Option<&'static str> {
    if title.contains("Claude Code") {
        return Some("claude");
    }
    if title.contains("opencode") {
        return Some("opencode");
    }
    if title.contains("Gemini") || title.contains('\u{25C7}') {
        return Some("gemini");
    }
    if title.contains('\u{03C0}') {
        return Some("pi");
    }
    if title.contains("Vibe") {
        return Some("vibe");
    }
    None
}

fn is_generic_interpreter(stem: &str) -> bool {
    let lower = stem.to_ascii_lowercase();
    GENERIC_INTERPRETERS.iter().any(|i| *i == lower)
}

fn is_version_string(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut has_dot = false;
    let mut prev_dot = true;
    for c in s.chars() {
        if c == '.' {
            if prev_dot {
                return false;
            }
            has_dot = true;
            prev_dot = true;
        } else if c.is_ascii_digit() {
            prev_dot = false;
        } else {
            return false;
        }
    }
    has_dot && !prev_dot
}

fn command_stem(command: &str) -> String {
    let token = command.split_whitespace().next().unwrap_or("");
    Path::new(token)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(token)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(cmd: &str, title: &str) -> Option<String> {
        classify_agent_kind(Some(cmd), Some(title))
    }

    #[test]
    fn version_string_matches_claude() {
        assert_eq!(classify("2.1.118", ""), Some("claude".into()));
        assert_eq!(classify("2.1.111", "✳ task"), Some("claude".into()));
        assert_eq!(classify("3.0.0.1", ""), Some("claude".into()));
    }

    #[test]
    fn codex_truncated_binary_matches() {
        assert_eq!(classify("codex-aarch64-a", ""), Some("codex".into()));
        assert_eq!(classify("codex", ""), Some("codex".into()));
    }

    #[test]
    fn opencode_exact_command() {
        assert_eq!(classify("opencode", "⠹ opencode"), Some("opencode".into()));
    }

    #[test]
    fn kiro_and_copilot_match() {
        assert_eq!(classify("kiro-cli", ""), Some("kiro-cli".into()));
        assert_eq!(classify("copilot", ""), Some("copilot".into()));
    }

    #[test]
    fn direct_stem_matches_for_known_binaries() {
        assert_eq!(classify("claude", ""), Some("claude".into()));
        assert_eq!(classify("gemini", ""), Some("gemini".into()));
        assert_eq!(classify("pi", ""), Some("pi".into()));
        assert_eq!(classify("vibe", ""), Some("vibe".into()));
    }

    #[test]
    fn absolute_path_is_normalized() {
        assert_eq!(classify("/usr/local/bin/claude", ""), Some("claude".into()));
        assert_eq!(classify("/opt/codex-aarch64-a", ""), Some("codex".into()));
    }

    #[test]
    fn node_with_claude_title() {
        assert_eq!(
            classify("node", "Claude Code 2.1.0 - foo"),
            Some("claude".into())
        );
    }

    #[test]
    fn node_with_gemini_title() {
        assert_eq!(
            classify("node", "\u{25C7}  Ready (sidebar-templates)"),
            Some("gemini".into())
        );
        assert_eq!(classify("node", "Gemini - working"), Some("gemini".into()));
    }

    #[test]
    fn node_with_pi_title() {
        assert_eq!(
            classify("node", "\u{03C0} - sidebar-templates"),
            Some("pi".into())
        );
    }

    #[test]
    fn python_with_vibe_title() {
        assert_eq!(classify("Python", "Vibe"), Some("vibe".into()));
        assert_eq!(classify("python3", "Vibe agent"), Some("vibe".into()));
    }

    #[test]
    fn opencode_via_node_title() {
        assert_eq!(
            classify("node", "⠹ opencode session"),
            Some("opencode".into())
        );
    }

    #[test]
    fn empty_command_returns_none() {
        assert_eq!(classify_agent_kind(None, None), None);
        assert_eq!(classify("", ""), None);
        assert_eq!(classify("", "Vibe"), None);
    }

    #[test]
    fn unknown_command_returns_none() {
        assert_eq!(classify("zsh", ""), None);
        assert_eq!(classify("vim", "some title"), None);
        // Bare prefix collisions with "codex" must not match.
        assert_eq!(classify("codexploitation", ""), None);
        assert_eq!(classify("codex2", ""), None);
    }

    #[test]
    fn generic_interpreter_no_matching_title_returns_none() {
        assert_eq!(classify("node", "random title"), None);
        assert_eq!(classify("Python", "no match"), None);
    }

    #[test]
    fn version_string_negative_cases() {
        assert!(!is_version_string(""));
        assert!(!is_version_string("2"));
        assert!(!is_version_string("2."));
        assert!(!is_version_string(".2"));
        assert!(!is_version_string("2..1"));
        assert!(!is_version_string("2.1a"));
        assert!(is_version_string("2.1"));
        assert!(is_version_string("2.1.118"));
    }
}
