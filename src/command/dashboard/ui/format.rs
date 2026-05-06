//! Formatting helpers for dashboard UI rendering.

use ratatui::style::{Modifier, Style};

/// Truncate a string to max_len characters, appending ellipsis if truncated.
pub fn truncate(s: &str, max_len: usize) -> String {
    if s.chars().count() > max_len {
        s.chars().take(max_len - 1).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

use crate::git::GitStatus;
use crate::github::PrSummary;
use crate::nerdfont;
use crate::ui::pr_status::{PrStatusOptions, format_pr_details as shared_format_pr_details};

use super::super::spinner::SPINNER_FRAMES;
use super::theme::ThemePalette;

/// Format git status for the Git column: base branch, diff stats, then indicators
/// Format: "→branch +N -M 󰏫 +X -Y 󰀪 ↑A ↓B"
/// When there are uncommitted changes that differ from total, branch totals are dimmed
pub fn format_git_status(
    status: Option<&GitStatus>,
    spinner_frame: u8,
    palette: &ThemePalette,
) -> Vec<(String, Style)> {
    let icons = nerdfont::git_icons();

    if let Some(status) = status {
        let mut spans: Vec<(String, Style)> = Vec::new();
        let has_uncommitted =
            status.uncommitted_added > 0 || status.uncommitted_removed > 0 || status.is_dirty;

        // Check if uncommitted equals total (all changes are uncommitted, nothing committed yet)
        let all_uncommitted = status.uncommitted_added == status.lines_added
            && status.uncommitted_removed == status.lines_removed;

        // Rebase indicator (shown first, before everything else)
        if status.is_rebasing {
            spans.push((
                icons.rebase.to_string(),
                Style::default().fg(palette.warning),
            ));
        }

        // Base branch (dimmed) - only show if not default (main/master)
        if !status.base_branch.is_empty()
            && status.base_branch != "main"
            && status.base_branch != "master"
        {
            spans.push((
                format!("→{}", status.base_branch),
                Style::default().fg(palette.dimmed),
            ));
        }

        // Always dim branch totals (historical), always bright uncommitted (active work)
        // - Clean: dim branch totals only
        // - All uncommitted: icon + bright uncommitted only
        // - Mixed: dim branch totals + icon + bright uncommitted
        if has_uncommitted && all_uncommitted {
            // All changes are uncommitted - show icon + bright numbers only
            if !spans.is_empty() {
                spans.push((" ".to_string(), Style::default()));
            }
            spans.push((icons.diff.to_string(), Style::default().fg(palette.accent)));

            if status.uncommitted_added > 0 {
                spans.push((" ".to_string(), Style::default()));
                spans.push((
                    format!("+{}", status.uncommitted_added),
                    Style::default().fg(palette.success),
                ));
            }
            if status.uncommitted_removed > 0 {
                spans.push((" ".to_string(), Style::default()));
                spans.push((
                    format!("-{}", status.uncommitted_removed),
                    Style::default().fg(palette.danger),
                ));
            }
        } else {
            // Either clean or mixed - show dim branch totals
            if status.lines_added > 0 {
                if !spans.is_empty() {
                    spans.push((" ".to_string(), Style::default()));
                }
                spans.push((
                    format!("+{}", status.lines_added),
                    Style::default()
                        .fg(palette.success)
                        .add_modifier(Modifier::DIM),
                ));
            }
            if status.lines_removed > 0 {
                if !spans.is_empty() {
                    spans.push((" ".to_string(), Style::default()));
                }
                spans.push((
                    format!("-{}", status.lines_removed),
                    Style::default()
                        .fg(palette.danger)
                        .add_modifier(Modifier::DIM),
                ));
            }

            // If there are uncommitted changes, show icon + bright uncommitted
            if has_uncommitted {
                if !spans.is_empty() {
                    spans.push((" ".to_string(), Style::default()));
                }
                spans.push((icons.diff.to_string(), Style::default().fg(palette.accent)));

                if status.uncommitted_added > 0 {
                    spans.push((" ".to_string(), Style::default()));
                    spans.push((
                        format!("+{}", status.uncommitted_added),
                        Style::default().fg(palette.success),
                    ));
                }
                if status.uncommitted_removed > 0 {
                    spans.push((" ".to_string(), Style::default()));
                    spans.push((
                        format!("-{}", status.uncommitted_removed),
                        Style::default().fg(palette.danger),
                    ));
                }
            }
        }

        // Conflict indicator
        if status.has_conflict {
            if !spans.is_empty() {
                spans.push((" ".to_string(), Style::default()));
            }
            spans.push((
                icons.conflict.to_string(),
                Style::default().fg(palette.danger),
            ));
        }

        // Ahead/behind upstream
        if status.ahead > 0 {
            if !spans.is_empty() {
                spans.push((" ".to_string(), Style::default()));
            }
            spans.push((
                format!("↑{}", status.ahead),
                Style::default().fg(palette.info),
            ));
        }
        if status.behind > 0 {
            if !spans.is_empty() {
                spans.push((" ".to_string(), Style::default()));
            }
            spans.push((
                format!("↓{}", status.behind),
                Style::default().fg(palette.warning),
            ));
        }

        if spans.is_empty() {
            vec![("-".to_string(), Style::default().fg(palette.dimmed))]
        } else {
            spans
        }
    } else {
        // No status yet - show spinner
        let frame = SPINNER_FRAMES[spinner_frame as usize % SPINNER_FRAMES.len()];
        vec![(frame.to_string(), Style::default().fg(palette.dimmed))]
    }
}

/// Format PR status as styled spans for dashboard display
pub fn format_pr_status(
    pr: Option<&PrSummary>,
    show_check_counts: bool,
    spinner_frame: u8,
    palette: &ThemePalette,
) -> Vec<(String, Style)> {
    crate::ui::pr_status::format_pr_status(
        pr,
        PrStatusOptions {
            include_number: true,
            show_check_counts,
            none_placeholder: Some("-"),
            is_stale: false,
        },
        spinner_frame,
        palette,
    )
}

/// Returns minimal PR detail spans for the preview title.
/// - Pending: "◷ 12m" (dimmed)
/// - Failure: "× lint-check" (danger color)
/// - Success/None: empty
pub fn format_pr_details(
    pr: &PrSummary,
    spinner_frame: u8,
    palette: &ThemePalette,
) -> Vec<ratatui::text::Span<'static>> {
    shared_format_pr_details(pr, spinner_frame, palette)
}
