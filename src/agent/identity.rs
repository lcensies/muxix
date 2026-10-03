//! Agent identity classification.
//!
//! Classifies an agent pane by combining tmux's `pane_current_command` with
//! the pane title. Some agents report a version string (Claude Code: "2.1.118"),
//! a truncated binary name (Codex: "codex-aarch64-a"), or run as a generic
//! interpreter (Gemini, Pi, Vibe). Stem-based profile resolution alone misses
//! these, so the result of `classify_agent_kind` is cached on `AgentState`
//! once it becomes non-None and reused by the sidebar render path.
//!
//! The canonical string form (e.g. "claude", "kiro-cli") matches the existing
//! `AgentProfile::name` so the sidebar can look up the corresponding profile.

use ratatui::style::Color;
use std::path::Path;

const GENERIC_INTERPRETERS: &[&str] = &["node", "python", "python3", "bun", "deno"];

/// Canonical set of agents the sidebar knows how to render.
///
/// Keeping per-variant metadata (icon, label) on this enum forces a compile
/// error in every consumer when a new variant is added, instead of silently
/// falling through to a generic default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
    OpenCode,
    Gemini,
    Pi,
    Omp,
    KiroCli,
    Vibe,
    Copilot,
}

impl AgentKind {
    /// Canonical string form. Matches `AgentProfile::name` and is the value
    /// persisted in `AgentState::agent_kind`.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::OpenCode => "opencode",
            AgentKind::Gemini => "gemini",
            AgentKind::Pi => "pi",
            AgentKind::Omp => "omp",
            AgentKind::KiroCli => "kiro-cli",
            AgentKind::Vibe => "vibe",
            AgentKind::Copilot => "copilot",
        }
    }

    /// Parse the canonical string form. Round-trips with [`AgentKind::as_str`].
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "claude" => Some(AgentKind::Claude),
            "codex" => Some(AgentKind::Codex),
            "opencode" => Some(AgentKind::OpenCode),
            "gemini" => Some(AgentKind::Gemini),
            "pi" => Some(AgentKind::Pi),
            "omp" => Some(AgentKind::Omp),
            "kiro-cli" => Some(AgentKind::KiroCli),
            "vibe" => Some(AgentKind::Vibe),
            "copilot" => Some(AgentKind::Copilot),
            _ => None,
        }
    }

    /// Default sidebar icon. Exhaustive match: adding a variant forces an
    /// update here.
    pub fn default_icon(self) -> &'static str {
        match self {
            AgentKind::Claude => "CC",
            AgentKind::Codex => "CX",
            AgentKind::OpenCode => "OC",
            AgentKind::Gemini => "G",
            AgentKind::Pi => "π",
            AgentKind::Omp => "ω",
            AgentKind::KiroCli => "K",
            AgentKind::Vibe => "V",
            AgentKind::Copilot => "CP",
        }
    }

    /// Default sidebar label. Exhaustive match: adding a variant forces an
    /// update here.
    pub fn default_label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude",
            AgentKind::Codex => "Codex",
            AgentKind::OpenCode => "OpenCode",
            AgentKind::Gemini => "Gemini",
            AgentKind::Pi => "Pi",
            AgentKind::Omp => "oh-my-pi",
            AgentKind::KiroCli => "Kiro",
            AgentKind::Vibe => "Vibe",
            AgentKind::Copilot => "Copilot",
        }
    }

    /// Default sidebar icon foreground color. Brand-true mid-luminance hex
    /// values that read on both shipped light and dark palettes.
    ///
    /// Sources: Anthropic, OpenAI, Google, GitHub, Mistral brand sheets;
    /// Pi sampled from product UI. `None` means "fall through to
    /// `palette.text`".
    pub fn default_color(self) -> Option<Color> {
        Some(match self {
            AgentKind::Claude => Color::Rgb(0xd9, 0x77, 0x57),
            AgentKind::Codex => Color::Rgb(0x10, 0xa3, 0x7f),
            AgentKind::Gemini => Color::Rgb(0x07, 0x8e, 0xfa),
            AgentKind::Copilot => Color::Rgb(0x89, 0x57, 0xe5),
            AgentKind::Vibe => Color::Rgb(0xff, 0x82, 0x05),
            AgentKind::Pi => Color::Rgb(0x96, 0xbb, 0xb5),
            AgentKind::Omp => Color::Rgb(0x96, 0xbb, 0xb5),
            AgentKind::OpenCode => Color::Blue,
            AgentKind::KiroCli => return None,
        })
    }
}

/// Classify an agent pane using its foreground command and pane title.
///
/// Returns the canonical profile name (e.g. "claude") or `None` if no rule
/// matches. Callers cache the first non-None result to avoid re-classifying
/// on every tick.
pub fn classify_agent_kind(command: Option<&str>, pane_title: Option<&str>) -> Option<String> {
    classify_agent_kind_enum(command, pane_title).map(|k| k.as_str().to_string())
}

fn classify_agent_kind_enum(command: Option<&str>, pane_title: Option<&str>) -> Option<AgentKind> {
    let raw = command.unwrap_or("").trim();
    let stem = command_stem(raw);

    if let Some(kind) = classify_by_command(raw, &stem) {
        return Some(kind);
    }

    if is_generic_interpreter(&stem)
        && let Some(kind) = classify_by_title(pane_title.unwrap_or(""))
    {
        return Some(kind);
    }

    None
}

fn classify_by_command(raw: &str, stem: &str) -> Option<AgentKind> {
    if stem.is_empty() {
        return None;
    }

    if is_version_string(stem) || is_version_string(raw) {
        return Some(AgentKind::Claude);
    }

    if stem == "codex" || stem.starts_with("codex-") {
        return Some(AgentKind::Codex);
    }

    AgentKind::from_str(stem)
}

fn classify_by_title(title: &str) -> Option<AgentKind> {
    if title.contains("Claude Code") {
        return Some(AgentKind::Claude);
    }
    if title.contains("opencode") {
        return Some(AgentKind::OpenCode);
    }
    if title.contains("Gemini") || title.contains('\u{25C7}') {
        return Some(AgentKind::Gemini);
    }
    if title.contains('\u{03C0}') {
        return Some(AgentKind::Pi);
    }
    if title.contains("Vibe") {
        return Some(AgentKind::Vibe);
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
    let stem = Path::new(token)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(token);
    normalize_wrapper_name(stem)
}

/// Strip launcher-wrapper decoration from a binary name.
///
/// Nix wraps executables as `.claude-wrapped`, and tmux truncates
/// `pane_current_command` to 15 characters — so the suffix frequently arrives
/// clipped (`.opencode-wrapp`). Matching on `-wrap` rather than the full
/// `-wrapped` handles both. Without this, every agent on a Nix system is
/// unclassifiable.
fn normalize_wrapper_name(name: &str) -> String {
    let name = name.strip_prefix('.').unwrap_or(name);
    match name.rfind("-wrap") {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

/// Detect the agent this workmux invocation is running *inside*, so a spawned
/// worktree can inherit its parent's agent instead of the configured default.
///
/// Signals, highest priority first:
/// 1. `WORKMUX_AGENT` — explicit override, forces an agent for a whole subtree.
/// 2. The current tmux pane's foreground command and title.
/// 3. A walk up the process tree.
///
/// Returns the canonical profile name (e.g. `"claude"`), or `None` when not
/// running under a recognizable agent.
pub fn detect_parent_agent() -> Option<String> {
    if let Ok(explicit) = std::env::var("WORKMUX_AGENT")
        && !explicit.trim().is_empty()
    {
        return Some(explicit.trim().to_string());
    }

    if let Some(kind) = detect_from_tmux_pane() {
        return Some(kind.as_str().to_string());
    }

    detect_from_ancestry().map(|k| k.as_str().to_string())
}

/// Classify the current tmux pane by its foreground command and title.
fn detect_from_tmux_pane() -> Option<AgentKind> {
    let out = std::process::Command::new("tmux")
        .args(["display-message", "-p", "#{pane_current_command}\t#{pane_title}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.trim_end_matches('\n');
    let (command, title) = line.split_once('\t')?;
    classify_agent_kind_enum(Some(command), Some(title))
}

/// Walk up the process tree, classifying each ancestor.
///
/// Catches the case where the pane's foreground command is a shell (workmux
/// invoked from an agent's shell tool) but an agent sits further up. Uses `ps`
/// rather than `/proc` so it works on macOS too.
fn detect_from_ancestry() -> Option<AgentKind> {
    const MAX_DEPTH: usize = 12;
    let mut pid = std::process::id();

    for _ in 0..MAX_DEPTH {
        let out = std::process::Command::new("ps")
            .args(["-o", "ppid=,comm=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let line = text.trim();
        if line.is_empty() {
            return None;
        }
        let (ppid_str, comm) = line.split_once(char::is_whitespace)?;

        if let Some(kind) = classify_agent_kind_enum(Some(comm.trim()), None) {
            return Some(kind);
        }

        pid = ppid_str.trim().parse().ok()?;
        if pid <= 1 {
            return None;
        }
    }
    None
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

    /// Nix wraps binaries as `.claude-wrapped`, and tmux truncates
    /// `pane_current_command` to 15 chars — so the suffix often arrives
    /// clipped. Both forms must still classify.
    #[test]
    fn nix_wrapper_names_are_normalized() {
        assert_eq!(classify(".claude-wrapped", ""), Some("claude".into()));
        assert_eq!(classify(".opencode-wrapped", ""), Some("opencode".into()));
        // tmux truncation to 15 characters
        assert_eq!(classify(".opencode-wrapp", ""), Some("opencode".into()));
        assert_eq!(classify(".gemini-wrap", ""), Some("gemini".into()));
        assert_eq!(
            classify("/nix/store/abc-claude-code-2.1.220/bin/.claude-wrapped", ""),
            Some("claude".into())
        );
    }

    #[test]
    fn wrapper_normalization_leaves_ordinary_names_alone() {
        assert_eq!(normalize_wrapper_name("claude"), "claude");
        assert_eq!(normalize_wrapper_name("kiro-cli"), "kiro-cli");
        // A leading dot alone is not a wrapper marker, but is still stripped
        // so dotfile-style launchers classify.
        assert_eq!(normalize_wrapper_name(".claude"), "claude");
        // Nothing before the marker: leave it be rather than produce "".
        assert_eq!(normalize_wrapper_name("-wrapped"), "-wrapped");
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

    /// Every variant has a non-empty icon and label; the classifier produces
    /// a string that round-trips back to the same variant. Catches forgetting
    /// to fill in metadata or string form when adding a variant.
    #[test]
    fn every_variant_has_metadata_and_round_trips() {
        let all = [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::OpenCode,
            AgentKind::Gemini,
            AgentKind::Pi,
            AgentKind::Omp,
            AgentKind::KiroCli,
            AgentKind::Vibe,
            AgentKind::Copilot,
        ];
        for kind in all {
            assert!(!kind.default_icon().is_empty(), "{:?} icon empty", kind);
            assert!(!kind.default_label().is_empty(), "{:?} label empty", kind);
            assert_eq!(AgentKind::from_str(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn from_str_rejects_unknown() {
        assert_eq!(AgentKind::from_str(""), None);
        assert_eq!(AgentKind::from_str("not-a-profile"), None);
        assert_eq!(AgentKind::from_str("Claude"), None); // case-sensitive
    }
}
