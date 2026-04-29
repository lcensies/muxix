//! Per-row context: pre-computed token values for a single agent row.

use ratatui::style::{Color, Modifier, Style};

use crate::agent_display::{extract_project_name, extract_worktree_name, sanitize_pane_title};
use crate::agent_identity::AgentKind;
use crate::git::GitStatus;
use crate::multiplexer::agent::resolve_profile_for_display;
use crate::multiplexer::{AgentPane, AgentStatus};
use crate::ui::theme::ThemePalette;

use super::super::app::SidebarApp;
use super::TokenId;

/// Pre-computed values for every piece of row metadata.
pub struct RowContext<'a> {
    pub agent: &'a AgentPane,
    /// Resolved primary label.
    pub primary: String,
    /// Resolved secondary label.
    pub secondary: String,
    /// Pane suffix like " (1)" when multiple agents share a window.
    pub pane_suffix: String,
    /// Compact elapsed string (e.g. "5:23", "2h", "1d").
    pub elapsed: String,
    /// Status icon parsed into styled spans.
    pub status_icon_spans: Vec<(String, Style)>,
    /// Foreground color extracted from status icon style.
    pub status_color: Color,
    /// Sanitized pane title, filtered against primary/secondary duplicates.
    pub pane_title: Option<String>,
    /// Git status for this agent's path.
    pub git_status: Option<&'a GitStatus>,
    /// Row flags.
    pub is_stale: bool,
    pub is_active: bool,
    pub is_selected: bool,
    /// Theme palette for style resolution.
    pub palette: &'a ThemePalette,
    /// Pre-resolved agent icon string (empty when no profile matches).
    pub agent_icon: String,
    /// Pre-resolved agent label string (empty when no profile matches).
    pub agent_label: String,
}

impl<'a> RowContext<'a> {
    pub fn build(
        app: &'a SidebarApp,
        agent: &'a AgentPane,
        idx: usize,
        pane_suffixes: &[String],
        now_secs: u64,
        selected_idx: Option<usize>,
    ) -> Self {
        let (primary, secondary) = app.resolve_agent_labels(agent);
        let pane_suffix = pane_suffixes[idx].clone();

        let is_sleeping = app.sleeping_pane_ids.contains(&agent.pane_id);
        let is_stale = agent
            .status_ts
            .map(|ts| now_secs.saturating_sub(ts) > app.stale_threshold_secs)
            .unwrap_or(false);
        let is_stale = is_sleeping
            || (is_stale
                && !matches!(
                    agent.status,
                    Some(AgentStatus::Working) | Some(AgentStatus::Waiting)
                ));
        let is_interrupted = app.interrupted_pane_ids.contains(&agent.pane_id);
        let is_active = app.host_agent_idx == Some(idx);
        let is_selected = selected_idx == Some(idx);

        let (status_icon_spans, status_icon_style) =
            super::super::ui::status_icon_and_style(app, agent.status, is_stale, is_interrupted);
        let status_color = status_icon_style.fg.unwrap_or(Color::Reset);

        let elapsed = if is_interrupted {
            String::new()
        } else {
            agent
                .status_ts
                .map(|ts| format_compact_elapsed(now_secs.saturating_sub(ts)))
                .unwrap_or_default()
        };

        let pane_title = build_pane_title(agent, &primary, &secondary, app.window_prefix());
        let git_status = app.git_statuses.get(&agent.path);
        let agent_icon = resolve_agent_icon(
            agent.agent_kind.as_deref(),
            agent.agent_command.as_deref(),
            &app.agent_icons,
        );
        let agent_label =
            resolve_agent_label(agent.agent_kind.as_deref(), agent.agent_command.as_deref());

        Self {
            agent,
            primary,
            secondary,
            pane_suffix,
            elapsed,
            status_icon_spans,
            status_color,
            pane_title,
            git_status,
            is_stale,
            is_active,
            is_selected,
            palette: &app.palette,
            agent_icon,
            agent_label,
        }
    }

    /// Resolve a token to its display string.
    pub fn resolve(&self, token: TokenId) -> String {
        match token {
            TokenId::Primary => self.primary.clone(),
            TokenId::Secondary => self.secondary.clone(),
            TokenId::Worktree => self.worktree_name(),
            TokenId::Project => self.project_name(),
            TokenId::Session => self.agent.session.clone(),
            TokenId::Window => self.agent.window_name.clone(),
            TokenId::PaneTitle => self.pane_title.clone().unwrap_or_default(),
            TokenId::AgentLabel => self.agent_label.clone(),
            TokenId::StatusIcon => self
                .status_icon_spans
                .iter()
                .map(|(t, _)| t.clone())
                .collect(),
            TokenId::AgentIcon => self.agent_icon.clone(),
            TokenId::PaneSuffix => self.pane_suffix.clone(),
            TokenId::Elapsed => self.elapsed.clone(),
            TokenId::GitStats => {
                // Composite token: empty string at resolution time;
                // layout engine calls git_stats_spans() for rendering.
                String::new()
            }
        }
    }

    /// Natural display width of a token's resolved text.
    pub fn natural_width(&self, token: TokenId) -> usize {
        match token {
            TokenId::StatusIcon => self
                .status_icon_spans
                .iter()
                .map(|(t, _)| display_width(t))
                .sum(),
            TokenId::AgentIcon => display_width(&self.agent_icon),
            TokenId::AgentLabel => display_width(&self.agent_label),
            TokenId::GitStats => {
                let (spans, width) = self.git_stats_spans(usize::MAX);
                let _ = spans;
                width
            }
            other => display_width(&self.resolve(other)),
        }
    }

    /// Render git stats with a given allocated width, returning styled spans and actual width.
    pub fn git_stats_spans(&self, allocated_width: usize) -> (Vec<(String, Style)>, usize) {
        match self.git_status {
            Some(status) => super::super::ui::format_sidebar_git_stats(
                Some(status),
                self.palette,
                self.is_stale,
                allocated_width,
            ),
            None => (Vec::new(), 0),
        }
    }

    /// Intrinsic style for a token (before state/selection post-pass).
    pub fn intrinsic_style(&self, token: TokenId) -> Style {
        if self.is_stale {
            return Style::default()
                .fg(self.palette.dimmed)
                .add_modifier(Modifier::DIM);
        }
        match token {
            TokenId::Primary if self.is_active => Style::default()
                .fg(self.palette.current_worktree_fg)
                .add_modifier(Modifier::BOLD),
            TokenId::Primary => Style::default().fg(self.palette.text),
            TokenId::Secondary => Style::default()
                .fg(self.palette.text)
                .add_modifier(Modifier::DIM),
            TokenId::PaneTitle => Style::default().fg(self.palette.dimmed),
            TokenId::PaneSuffix => Style::default().fg(self.palette.dimmed),
            TokenId::Elapsed => Style::default().fg(self.palette.text),
            TokenId::AgentLabel => Style::default().fg(self.palette.text),
            _ => Style::default().fg(self.palette.text),
        }
    }

    fn worktree_name(&self) -> String {
        let (wt, _) = extract_worktree_name(
            &self.agent.session,
            &self.agent.window_name,
            "",
            &self.agent.path,
        );
        wt
    }

    fn project_name(&self) -> String {
        extract_project_name(&self.agent.path)
    }
}

fn resolve_agent_label(agent_kind: Option<&str>, agent_command: Option<&str>) -> String {
    match effective_agent_kind(agent_kind, agent_command) {
        Some(kind) => kind.default_label().to_string(),
        None => String::new(),
    }
}

fn resolve_agent_icon(
    agent_kind: Option<&str>,
    agent_command: Option<&str>,
    agent_icons: &std::collections::BTreeMap<String, String>,
) -> String {
    let Some(kind) = effective_agent_kind(agent_kind, agent_command) else {
        return String::new();
    };
    if let Some(icon) = agent_icons.get(kind.as_str()) {
        return icon.clone();
    }
    kind.default_icon().to_string()
}

/// Prefer the cached classification; fall back to today's stem-based resolver.
///
/// A malformed, hand-edited, or future-version state file with an unknown
/// `agent_kind` falls through to the command-based resolver instead of
/// shadowing a perfectly good `agent_command` with a meaningless icon/label.
fn effective_agent_kind(
    agent_kind: Option<&str>,
    agent_command: Option<&str>,
) -> Option<AgentKind> {
    if let Some(kind) = agent_kind.and_then(AgentKind::from_str) {
        return Some(kind);
    }
    AgentKind::from_str(resolve_profile_for_display(agent_command).name())
}

fn build_pane_title(
    agent: &AgentPane,
    primary: &str,
    secondary: &str,
    window_prefix: &str,
) -> Option<String> {
    let title_worktree = extract_worktree_name(
        &agent.session,
        &agent.window_name,
        window_prefix,
        &agent.path,
    )
    .0;
    let title_project = extract_project_name(&agent.path);
    sanitize_pane_title(agent.pane_title.as_deref(), &title_worktree, &title_project)
        .filter(|t| *t != primary && *t != secondary)
        .map(|s| s.to_string())
}

fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
        .sum()
}

fn format_compact_elapsed(secs: u64) -> String {
    if secs < 3600 {
        format!("{}:{:02}", secs / 60, secs % 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn cached_kind_resolves_label_without_command() {
        // Command is a version string the stem-based resolver can't classify;
        // the cached kind must drive label/icon.
        assert_eq!(
            resolve_agent_label(Some("claude"), Some("2.1.118")),
            "Claude"
        );
    }

    #[test]
    fn cached_kind_renders_friendly_kiro_label() {
        assert_eq!(resolve_agent_label(Some("kiro-cli"), None), "Kiro");
    }

    #[test]
    fn cached_kind_renders_friendly_opencode_label() {
        assert_eq!(resolve_agent_label(Some("opencode"), None), "OpenCode");
    }

    #[test]
    fn unknown_cached_kind_falls_back_to_command() {
        // Defensive: malformed cache must not shadow a valid agent_command.
        let icons = BTreeMap::new();
        assert_eq!(
            resolve_agent_label(Some("not-a-profile"), Some("claude")),
            "Claude"
        );
        assert_eq!(
            resolve_agent_icon(Some("not-a-profile"), Some("claude"), &icons),
            "CC"
        );
    }

    #[test]
    fn no_cache_falls_back_to_today_behavior() {
        let icons = BTreeMap::new();
        assert_eq!(resolve_agent_label(None, Some("gemini")), "Gemini");
        assert_eq!(resolve_agent_icon(None, Some("gemini"), &icons), "G");
    }

    #[test]
    fn custom_icon_override_still_honored_with_cached_kind() {
        let mut icons = BTreeMap::new();
        icons.insert("claude".to_string(), "X".to_string());
        assert_eq!(
            resolve_agent_icon(Some("claude"), Some("2.1.118"), &icons),
            "X"
        );
    }
}
