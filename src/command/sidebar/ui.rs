//! Rendering for the sidebar TUI.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Padding};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_width::UnicodeWidthChar;

use crate::git::GitStatus;
use crate::multiplexer::{AgentPane, AgentStatus};
use crate::tmux_style;
use crate::ui::theme::ThemePalette;

use super::app::{SidebarApp, SidebarLayoutMode};
use super::template::context::RowContext;
use super::template::layout::{is_empty_line, render_line};

/// Compute pane suffixes like " (1)", " (2)" for agents sharing the same window.
fn compute_pane_suffixes(agents: &[AgentPane]) -> Vec<String> {
    let mut counts: HashMap<(&str, &str), usize> = HashMap::new();
    for agent in agents {
        *counts
            .entry((&agent.session, &agent.window_name))
            .or_default() += 1;
    }

    let mut positions: HashMap<(&str, &str), usize> = HashMap::new();
    agents
        .iter()
        .map(|agent| {
            let key = (agent.session.as_str(), agent.window_name.as_str());
            if counts[&key] > 1 {
                let pos = positions.entry(key).or_default();
                *pos += 1;
                format!("({})", pos)
            } else {
                String::new()
            }
        })
        .collect()
}

/// Format git diff stats for sidebar display, fitting within `available_width`.
/// Uses same colors as dashboard: DIM committed stats, bright uncommitted stats.
/// When `is_stale` is true, all colors are forced to dimmed.
///
/// Priority when space is limited:
/// 1. Uncommitted diff stats (bright +N -M with diff icon)
/// 2. Committed/branch diff stats (dimmed +N -M)
///
/// Returns pre-built spans (without background) and total display width.
pub(crate) fn format_sidebar_git_stats(
    status: Option<&GitStatus>,
    palette: &ThemePalette,
    is_stale: bool,
    available_width: usize,
) -> (Vec<(String, Style)>, usize) {
    let Some(status) = status else {
        return (vec![], 0);
    };

    let icons = crate::nerdfont::git_icons();

    // When stale, force all colors to dimmed
    let success = if is_stale {
        palette.dimmed
    } else {
        palette.success
    };
    let danger = if is_stale {
        palette.dimmed
    } else {
        palette.danger
    };
    let accent = if is_stale {
        palette.dimmed
    } else {
        palette.accent
    };

    let has_committed = status.lines_added > 0 || status.lines_removed > 0;
    let has_uncommitted =
        status.uncommitted_added > 0 || status.uncommitted_removed > 0 || status.is_dirty;

    // Same logic as dashboard: if all changes are uncommitted, skip the dimmed committed section
    let all_uncommitted = has_uncommitted
        && status.uncommitted_added == status.lines_added
        && status.uncommitted_removed == status.lines_removed;

    if !has_committed && !has_uncommitted && !status.is_rebasing {
        return (vec![], 0);
    }

    // Width of a set of spans: text widths + a single space between each pair.
    let calc_width = |spans: &[(String, Style)]| -> usize {
        if spans.is_empty() {
            return 0;
        }
        spans.iter().map(|(s, _)| display_width(s)).sum::<usize>() + spans.len() - 1
    };

    // Build rebase indicator (shown first, highest priority)
    let mut rebase_spans: Vec<(String, Style)> = Vec::new();
    if status.is_rebasing {
        let rebase_color = if is_stale {
            palette.dimmed
        } else {
            palette.warning
        };
        rebase_spans.push((icons.rebase.to_string(), Style::default().fg(rebase_color)));
    }

    // Build uncommitted spans (bright, with diff icon)
    let mut uncommitted_spans: Vec<(String, Style)> = Vec::new();
    if has_uncommitted {
        uncommitted_spans.push((icons.diff.to_string(), Style::default().fg(accent)));
        if status.uncommitted_added > 0 {
            uncommitted_spans.push((
                format!("+{}", status.uncommitted_added),
                Style::default().fg(success),
            ));
        }
        if status.uncommitted_removed > 0 {
            uncommitted_spans.push((
                format!("-{}", status.uncommitted_removed),
                Style::default().fg(danger),
            ));
        }
    }

    // Build committed spans (dimmed) - skip if all changes are uncommitted
    let mut committed_spans: Vec<(String, Style)> = Vec::new();
    if has_committed && !all_uncommitted {
        if status.lines_added > 0 {
            committed_spans.push((
                format!("+{}", status.lines_added),
                Style::default().fg(success).add_modifier(Modifier::DIM),
            ));
        }
        if status.lines_removed > 0 {
            committed_spans.push((
                format!("-{}", status.lines_removed),
                Style::default().fg(danger).add_modifier(Modifier::DIM),
            ));
        }
    }

    // Insert a single space between adjacent spans (no trailing space).
    let interleave = |spans: Vec<(String, Style)>| -> Vec<(String, Style)> {
        let mut out: Vec<(String, Style)> = Vec::with_capacity(spans.len() * 2);
        let mut first = true;
        for span in spans {
            if !first {
                out.push((" ".to_string(), Style::default()));
            }
            first = false;
            out.push(span);
        }
        out
    };

    // Try variants in priority order: full > drop committed > drop uncommitted > rebase only.
    let candidates: Vec<Vec<(String, Style)>> = vec![
        {
            let mut s = rebase_spans.clone();
            s.extend(committed_spans.clone());
            s.extend(uncommitted_spans.clone());
            s
        },
        {
            let mut s = rebase_spans.clone();
            s.extend(uncommitted_spans);
            s
        },
        rebase_spans,
    ];

    for spans in candidates {
        let width = calc_width(&spans);
        if width > 0 && width <= available_width {
            return (interleave(spans), width);
        }
    }
    (vec![], 0)
}

/// Width of an interleaved span list (text widths + 1 col per joiner space).
fn interleaved_width(spans: &[(String, Style)]) -> usize {
    if spans.is_empty() {
        return 0;
    }
    spans.iter().map(|(s, _)| display_width(s)).sum::<usize>() + spans.len() - 1
}

/// Insert a single space between adjacent spans (no trailing space).
fn interleave_spans(spans: Vec<(String, Style)>) -> Vec<(String, Style)> {
    let mut out: Vec<(String, Style)> = Vec::with_capacity(spans.len() * 2);
    let mut first = true;
    for span in spans {
        if !first {
            out.push((" ".to_string(), Style::default()));
        }
        first = false;
        out.push(span);
    }
    out
}

/// Pick the widest variant (in priority order) that fits `max_width`.
/// Variants are pre-interleave: each entry is a list of styled text fragments
/// that will be joined by a single space.
fn pick_fitting_variant(
    variants: Vec<Vec<(String, Style)>>,
    max_width: usize,
) -> (Vec<(String, Style)>, usize) {
    for raw in variants {
        let width = interleaved_width(&raw);
        if width > 0 && width <= max_width {
            return (interleave_spans(raw), width);
        }
    }
    (Vec::new(), 0)
}

/// Format the committed/branch-diff segment of git stats with self-fitting.
///
/// Variant ladder (widest first): `+N -M` → `+N` → `-M` → empty.
/// Returns empty when there are no committed changes or when all changes
/// are uncommitted (the composite hides committed in that case to avoid
/// duplicating the uncommitted numbers).
pub(crate) fn format_committed_spans(
    status: Option<&GitStatus>,
    palette: &ThemePalette,
    is_stale: bool,
    max_width: usize,
) -> (Vec<(String, Style)>, usize) {
    let Some(status) = status else {
        return (Vec::new(), 0);
    };

    let has_committed = status.lines_added > 0 || status.lines_removed > 0;
    let has_uncommitted =
        status.uncommitted_added > 0 || status.uncommitted_removed > 0 || status.is_dirty;
    let all_uncommitted = has_uncommitted
        && status.uncommitted_added == status.lines_added
        && status.uncommitted_removed == status.lines_removed;

    if !has_committed || all_uncommitted {
        return (Vec::new(), 0);
    }

    let success = if is_stale {
        palette.dimmed
    } else {
        palette.success
    };
    let danger = if is_stale {
        palette.dimmed
    } else {
        palette.danger
    };
    let style_a = Style::default().fg(success).add_modifier(Modifier::DIM);
    let style_r = Style::default().fg(danger).add_modifier(Modifier::DIM);

    let added = (status.lines_added > 0).then(|| (format!("+{}", status.lines_added), style_a));
    let removed =
        (status.lines_removed > 0).then(|| (format!("-{}", status.lines_removed), style_r));

    let mut variants: Vec<Vec<(String, Style)>> = Vec::new();
    match (&added, &removed) {
        (Some(a), Some(r)) => {
            variants.push(vec![a.clone(), r.clone()]);
            variants.push(vec![a.clone()]);
            variants.push(vec![r.clone()]);
        }
        (Some(a), None) => variants.push(vec![a.clone()]),
        (None, Some(r)) => variants.push(vec![r.clone()]),
        (None, None) => {}
    }

    pick_fitting_variant(variants, max_width)
}

/// Format the uncommitted/diff segment with self-fitting.
///
/// Variant ladder: `icon +N -M` → `icon +N` → `icon -M` → `icon` → empty.
pub(crate) fn format_uncommitted_spans(
    status: Option<&GitStatus>,
    palette: &ThemePalette,
    is_stale: bool,
    max_width: usize,
) -> (Vec<(String, Style)>, usize) {
    let Some(status) = status else {
        return (Vec::new(), 0);
    };

    let has_uncommitted =
        status.uncommitted_added > 0 || status.uncommitted_removed > 0 || status.is_dirty;
    if !has_uncommitted {
        return (Vec::new(), 0);
    }

    let icons = crate::nerdfont::git_icons();
    let success = if is_stale {
        palette.dimmed
    } else {
        palette.success
    };
    let danger = if is_stale {
        palette.dimmed
    } else {
        palette.danger
    };
    let accent = if is_stale {
        palette.dimmed
    } else {
        palette.accent
    };

    let icon = (icons.diff.to_string(), Style::default().fg(accent));
    let added = (status.uncommitted_added > 0).then(|| {
        (
            format!("+{}", status.uncommitted_added),
            Style::default().fg(success),
        )
    });
    let removed = (status.uncommitted_removed > 0).then(|| {
        (
            format!("-{}", status.uncommitted_removed),
            Style::default().fg(danger),
        )
    });

    let mut variants: Vec<Vec<(String, Style)>> = Vec::new();
    match (&added, &removed) {
        (Some(a), Some(r)) => {
            variants.push(vec![icon.clone(), a.clone(), r.clone()]);
            variants.push(vec![icon.clone(), a.clone()]);
            variants.push(vec![icon.clone(), r.clone()]);
        }
        (Some(a), None) => variants.push(vec![icon.clone(), a.clone()]),
        (None, Some(r)) => variants.push(vec![icon.clone(), r.clone()]),
        (None, None) => {} // dirty but no line counts: fall through to icon-only
    }
    variants.push(vec![icon.clone()]);

    pick_fitting_variant(variants, max_width)
}

/// Format the rebase indicator with self-fitting.
pub(crate) fn format_rebase_spans(
    status: Option<&GitStatus>,
    palette: &ThemePalette,
    is_stale: bool,
    max_width: usize,
) -> (Vec<(String, Style)>, usize) {
    let Some(status) = status else {
        return (Vec::new(), 0);
    };
    if !status.is_rebasing {
        return (Vec::new(), 0);
    }
    let icons = crate::nerdfont::git_icons();
    let color = if is_stale {
        palette.dimmed
    } else {
        palette.warning
    };
    let icon = (icons.rebase.to_string(), Style::default().fg(color));
    pick_fitting_variant(vec![vec![icon]], max_width)
}

/// Render the sidebar UI.
pub fn render_sidebar(f: &mut Frame, app: &mut SidebarApp) {
    let area = f.area();

    let padding = match app.layout_mode {
        // Compact mode: pad both sides for breathing room
        SidebarLayoutMode::Compact => Padding::new(1, 1, 0, 0),
        // Tile mode: stripe provides left edge, border is already excluded from inner area
        SidebarLayoutMode::Tiles => Padding::ZERO,
    };

    let block = Block::default().padding(padding);

    let inner = block.inner(area);
    f.render_widget(block, area);
    app.list_area = inner;

    match app.layout_mode {
        SidebarLayoutMode::Compact => render_compact_list(f, app, inner),
        SidebarLayoutMode::Tiles => render_tile_list(f, app, inner),
    }
}

/// Compact single-line-per-agent list (original layout).
fn render_compact_list(f: &mut Frame, app: &mut SidebarApp, area: Rect) {
    if app.agents.is_empty() {
        render_empty_state(f, app, area);
        return;
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let pane_suffixes = compute_pane_suffixes(&app.agents);
    let selected_idx = app.list_state.selected();
    let template = app.templates.compact.clone();
    let width = area.width as usize;

    let items: Vec<ListItem> = app
        .agents
        .iter()
        .enumerate()
        .map(|(idx, agent)| {
            let ctx = RowContext::build(app, agent, idx, &pane_suffixes, now_secs, selected_idx);
            let mut spans = render_line(&ctx, &template, width);

            // Post-pass: apply selection background
            if ctx.is_selected {
                for span in &mut spans {
                    span.style = span.style.bg(app.palette.highlight_row_bg);
                }
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let list = List::new(items).highlight_style(Style::default().bg(app.palette.highlight_row_bg));

    f.render_stateful_widget(list, area, &mut app.list_state);
}

/// Tile layout: variable-height cards per agent with status stripe.
fn render_tile_list(f: &mut Frame, app: &mut SidebarApp, area: Rect) {
    if app.agents.is_empty() {
        render_empty_state(f, app, area);
        return;
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let sep_width = area.width as usize;
    let selected_idx = app.list_state.selected();
    let agent_count = app.agents.len();
    let pane_suffixes = compute_pane_suffixes(&app.agents);
    let tile_templates: Vec<_> = app.templates.tiles.clone();
    let body_width = (area.width as usize).saturating_sub(6); // stripe(2) + icon(2) + gap(1) + right margin(1)

    let mut tile_heights = Vec::new();

    let items: Vec<ListItem> = app
        .agents
        .iter()
        .enumerate()
        .map(|(idx, agent)| {
            let ctx = RowContext::build(app, agent, idx, &pane_suffixes, now_secs, selected_idx);

            // Stripe color on all lines; stale forces dimmed
            let stripe_color = if ctx.is_stale {
                app.palette.dimmed
            } else {
                ctx.status_color
            };
            let stripe_style = Style::default().fg(stripe_color);

            let bg = if ctx.is_selected {
                Some(app.palette.highlight_row_bg)
            } else {
                None
            };

            let mut stripe_bg_style = stripe_style;
            if let Some(bg_color) = bg {
                stripe_bg_style = stripe_bg_style.bg(bg_color);
            }

            // Pad icon to fixed 2-column width
            let icon_cols: usize = ctx
                .status_icon_spans
                .iter()
                .map(|(t, _)| display_width(t))
                .sum();
            let icon_pad = if icon_cols < 2 {
                " ".repeat(2 - icon_cols)
            } else {
                String::new()
            };

            // Separator at the top (between tiles, not on first item)
            let mut lines = Vec::new();
            if idx > 0 {
                lines.push(Line::from(Span::styled(
                    "─".repeat(sep_width),
                    Style::default().fg(app.palette.border),
                )));
            }

            let mut visible_lines = 0;

            for (line_idx, template) in tile_templates.iter().enumerate() {
                // Drop empty lines
                if is_empty_line(template, &ctx) {
                    continue;
                }
                visible_lines += 1;

                let mut line_spans: Vec<Span> = vec![Span::styled("▌ ", stripe_bg_style)];

                // Chrome: icon column (status icon on line 1, blank on lines 2+)
                if line_idx == 0 {
                    for (text, style) in &ctx.status_icon_spans {
                        line_spans.push(Span::styled(text.clone(), *style));
                    }
                    line_spans.push(Span::raw(icon_pad.clone()));
                } else {
                    line_spans.push(Span::raw("  "));
                }

                // Chrome: gap
                line_spans.push(Span::raw(" "));

                // Body: template rendering
                let body_spans = render_line(&ctx, template, body_width);
                line_spans.extend(body_spans);

                // Right margin: 1 blank column so content doesn't touch the edge.
                line_spans.push(Span::raw(" "));

                // Post-pass: apply selection background
                if ctx.is_selected {
                    for span in &mut line_spans {
                        span.style = span.style.bg(app.palette.highlight_row_bg);
                    }
                }

                lines.push(Line::from(line_spans));
            }

            // If all lines were empty, render at least one blank line so the tile doesn't collapse
            if visible_lines == 0 {
                visible_lines = 1;
                lines.push(Line::from(vec![
                    Span::styled("▌ ", stripe_bg_style),
                    Span::raw("  "),
                    Span::raw(" "),
                    Span::raw(" ".repeat(body_width)),
                    Span::raw(" "),
                ]));
            }

            tile_heights.push(visible_lines);

            // Bottom separator after the last item
            if idx == agent_count - 1 {
                lines.push(Line::from(Span::styled(
                    "─".repeat(sep_width),
                    Style::default().fg(app.palette.border),
                )));
            }

            ListItem::new(lines)
        })
        .collect();

    app.tile_heights = tile_heights;

    // No highlight_style - background is baked into content lines to avoid highlighting separators
    let list = List::new(items);

    f.render_stateful_widget(list, area, &mut app.list_state);
}

/// Get the status icon as parsed styled spans and the base style for an agent.
///
/// Returns `(spans, base_style)` where `spans` contains tmux style codes parsed into
/// individual `(text, style)` pairs, and `base_style` is the fallback style (used for
/// stripe color, etc.).
pub(crate) fn status_icon_and_style(
    app: &SidebarApp,
    status: Option<AgentStatus>,
    is_stale: bool,
    is_interrupted: bool,
) -> (Vec<(String, Style)>, Style) {
    if is_stale {
        let style = Style::default().fg(app.palette.dimmed);
        return (vec![("💤".to_string(), style)], style);
    }
    if is_interrupted {
        let style = Style::default().fg(app.palette.dimmed);
        return (vec![("  ".to_string(), style)], style);
    }
    match status {
        Some(AgentStatus::Working) => {
            let base_style = Style::default().fg(app.palette.info);
            let spans = match &app.status_icons.working {
                Some(custom) => tmux_style::parse_tmux_styles(custom, base_style),
                None => {
                    let frames: &[&str] =
                        &["⠋⠙", "⠙⠹", "⠹⠸", "⠸⠼", "⠼⠴", "⠴⠦", "⠦⠧", "⠧⠇", "⠇⠏", "⠏⠋"];
                    vec![(
                        frames[app.spinner_frame as usize % frames.len()].to_string(),
                        base_style,
                    )]
                }
            };
            (spans, base_style)
        }
        Some(AgentStatus::Waiting) => {
            let base_style = Style::default().fg(app.palette.accent);
            let spans = tmux_style::parse_tmux_styles(app.status_icons.waiting(), base_style);
            (spans, base_style)
        }
        Some(AgentStatus::Done) => {
            let base_style = Style::default().fg(app.palette.success);
            let spans = tmux_style::parse_tmux_styles(app.status_icons.done(), base_style);
            (spans, base_style)
        }
        None => {
            let style = Style::default().fg(app.palette.dimmed);
            (vec![("  ".to_string(), style)], style)
        }
    }
}

fn render_empty_state(f: &mut Frame, app: &SidebarApp, area: Rect) {
    let text = Line::from(Span::styled(
        "No agents running",
        Style::default().fg(app.palette.dimmed),
    ))
    .alignment(Alignment::Center);
    let y = area.y + area.height / 2;
    let centered = Rect::new(area.x, y, area.width, 1);
    f.render_widget(text, centered);
}

/// Get the display width of a string, counting wide chars as 2.
pub(crate) fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
        .sum()
}

#[cfg(test)]
mod tests {
    use crate::agent_display::{sanitize_pane_title, strip_oc_title_prefix};

    #[test]
    fn strips_oc_prefixes() {
        assert_eq!(
            strip_oc_title_prefix("OC | Investigating..."),
            "Investigating..."
        );
        assert_eq!(
            strip_oc_title_prefix("OC | OC | Investigating..."),
            "Investigating..."
        );
    }

    #[test]
    fn keeps_non_agent_pipe_titles() {
        assert_eq!(
            strip_oc_title_prefix("Build | Investigating..."),
            "Build | Investigating..."
        );
        assert_eq!(
            strip_oc_title_prefix("Claude Code | Investigating..."),
            "Claude Code | Investigating..."
        );
    }

    #[test]
    fn sanitize_pane_title_drops_empty_after_prefix_strip() {
        assert_eq!(
            sanitize_pane_title(Some("OC |"), "worktree", "project"),
            None
        );
    }

    #[test]
    fn sanitize_pane_title_strips_icons_and_agent_prefixes() {
        assert_eq!(
            sanitize_pane_title(Some("⠋⠙ OC | Investigating..."), "worktree", "project"),
            Some("Investigating...")
        );
    }
}
