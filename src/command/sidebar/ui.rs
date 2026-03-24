//! Rendering for the sidebar TUI.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Padding};
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_width::UnicodeWidthChar;

use crate::command::dashboard::agent::{extract_project_name, extract_worktree_name};
use crate::multiplexer::AgentStatus;

use super::app::{SidebarApp, SidebarLayoutMode};

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

    match app.layout_mode {
        SidebarLayoutMode::Compact => render_compact_list(f, app, inner),
        SidebarLayoutMode::Tiles => render_tile_list(f, app, inner),
    }
}

/// Compact single-line-per-agent list (original layout).
fn render_compact_list(f: &mut Frame, app: &mut SidebarApp, area: Rect) {
    if app.agents.is_empty() {
        let empty_line = Line::from(Span::styled(
            "No agents",
            Style::default().fg(app.palette.dimmed),
        ));
        let list = List::new(vec![ListItem::new(empty_line)]);
        f.render_widget(list, area);
        return;
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let items: Vec<ListItem> = app
        .agents
        .iter()
        .map(|agent| {
            let worktree_name = app.display_name(agent);

            let is_stale = agent
                .status_ts
                .map(|ts| now_secs.saturating_sub(ts) > app.stale_threshold_secs)
                .unwrap_or(false);

            // Status icon
            let (icon, icon_style) = status_icon_and_style(app, agent.status, is_stale);

            // Elapsed time
            let elapsed = agent
                .status_ts
                .map(|ts| format_compact_elapsed(now_secs.saturating_sub(ts)))
                .unwrap_or_default();

            // Pad icon to fixed 2-column width so emoji and spinners align
            let icon_cols = display_width(&icon);
            let icon_pad = if icon_cols < 2 {
                " ".repeat(2 - icon_cols)
            } else {
                String::new()
            };

            // Calculate available width for the name
            // Layout: "{icon}{pad} {name} {elapsed}"
            let elapsed_width = elapsed.len();
            // Reserve: 2 (icon slot) + 1 (space) + 1 (space) + elapsed
            let reserved = 2 + 1 + 1 + elapsed_width;
            let name_width = (area.width as usize).saturating_sub(reserved);

            let display_name = truncate_to_width(&worktree_name, name_width);
            let padding = name_width.saturating_sub(display_width(&display_name));

            let is_active = app
                .active_window
                .as_ref()
                .is_some_and(|w| w == &agent.window_name)
                && app
                    .active_session
                    .as_ref()
                    .is_some_and(|s| s == &agent.session);

            let name_style = if is_stale {
                Style::default()
                    .fg(app.palette.dimmed)
                    .add_modifier(Modifier::DIM)
            } else if is_active {
                Style::default()
                    .fg(app.palette.current_worktree_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.palette.text)
            };

            let elapsed_style = Style::default().fg(app.palette.dimmed);

            let line = Line::from(vec![
                Span::styled(icon, icon_style),
                Span::raw(icon_pad),
                Span::raw(" "),
                Span::styled(display_name, name_style),
                Span::raw(" ".repeat(padding)),
                Span::raw(" "),
                Span::styled(elapsed, elapsed_style),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).highlight_style(Style::default().bg(app.palette.highlight_row_bg));

    f.render_stateful_widget(list, area, &mut app.list_state);
}

/// Tile layout: variable-height cards per agent with status stripe.
fn render_tile_list(f: &mut Frame, app: &mut SidebarApp, area: Rect) {
    if app.agents.is_empty() {
        let empty_line = Line::from(Span::styled(
            "No agents",
            Style::default().fg(app.palette.dimmed),
        ));
        let list = List::new(vec![ListItem::new(empty_line)]);
        f.render_widget(list, area);
        return;
    }

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let sep_width = area.width as usize;
    let selected_idx = app.list_state.selected();

    let items: Vec<ListItem> = app
        .agents
        .iter()
        .enumerate()
        .map(|(idx, agent)| {
            let is_selected = selected_idx == Some(idx);
            let project = extract_project_name(&agent.path);
            let (worktree, is_main) =
                extract_worktree_name(&agent.session, &agent.window_name, app.window_prefix());

            let display_worktree = if is_main {
                project.clone()
            } else {
                worktree.clone()
            };

            let is_stale = agent
                .status_ts
                .map(|ts| now_secs.saturating_sub(ts) > app.stale_threshold_secs)
                .unwrap_or(false);

            let is_active = app
                .active_window
                .as_ref()
                .is_some_and(|w| w == &agent.window_name)
                && app
                    .active_session
                    .as_ref()
                    .is_some_and(|s| s == &agent.session);

            // Status icon and color
            let (icon, icon_style) = status_icon_and_style(app, agent.status, is_stale);
            let status_color = icon_style.fg.unwrap_or(ratatui::style::Color::Reset);

            // Stripe color on all lines; stale forces dimmed
            let stripe_color = if is_stale {
                app.palette.dimmed
            } else {
                status_color
            };
            let stripe_style = Style::default().fg(stripe_color);

            // Elapsed time
            let elapsed = agent
                .status_ts
                .map(|ts| format_compact_elapsed(now_secs.saturating_sub(ts)))
                .unwrap_or_default();

            // Pad icon to fixed 2-column width
            let icon_cols = display_width(&icon);
            let icon_pad = if icon_cols < 2 {
                " ".repeat(2 - icon_cols)
            } else {
                String::new()
            };

            // Line 1 content width: area - stripe(2) - icon(2+pad) - space(1) - space(1) - elapsed - space(1)
            let line1_name_width =
                (area.width as usize).saturating_sub(2 + 2 + 1 + 1 + 1 + elapsed.len());
            // Body lines indent to align with worktree name: icon(2) + gap(1) = 3
            let body_indent = "   ";
            let body_width = (area.width as usize).saturating_sub(2 + body_indent.len());

            let display_name = truncate_with_ellipsis(&display_worktree, line1_name_width);
            let name_padding = line1_name_width.saturating_sub(display_width(&display_name));

            // Styles - apply highlight background on selected tiles' content lines
            let bg = if is_selected {
                Some(app.palette.highlight_row_bg)
            } else {
                None
            };

            let mut name_style = if is_stale {
                Style::default()
                    .fg(app.palette.dimmed)
                    .add_modifier(Modifier::DIM)
            } else if is_active {
                Style::default()
                    .fg(app.palette.current_worktree_fg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(app.palette.text)
            };

            let mut body_style = if is_stale {
                Style::default()
                    .fg(app.palette.dimmed)
                    .add_modifier(Modifier::DIM)
            } else {
                Style::default().fg(app.palette.dimmed)
            };

            let mut elapsed_style = Style::default().fg(app.palette.dimmed);

            let mut stripe_bg_style = stripe_style;
            let mut icon_bg_style = icon_style;

            if let Some(bg_color) = bg {
                name_style = name_style.bg(bg_color);
                body_style = body_style.bg(bg_color);
                elapsed_style = elapsed_style.bg(bg_color);
                stripe_bg_style = stripe_bg_style.bg(bg_color);
                icon_bg_style = icon_bg_style.bg(bg_color);
            }

            // Padding style (spaces that need background in selected state)
            let pad_style = bg.map(|c| Style::default().bg(c)).unwrap_or_default();

            // Line 1: ▌ icon  worktree-name    elapsed
            // Compute used width to pad trailing space
            let line1_used = 2
                + display_width(&icon)
                + (2usize.saturating_sub(icon_cols))
                + 1
                + display_width(&display_name)
                + name_padding
                + 1
                + elapsed.len();
            let line1_trail = (area.width as usize).saturating_sub(line1_used);

            let line1 = Line::from(vec![
                Span::styled("▌ ", stripe_bg_style),
                Span::styled(icon, icon_bg_style),
                Span::styled(icon_pad, pad_style),
                Span::styled(" ", pad_style),
                Span::styled(display_name, name_style),
                Span::styled(" ".repeat(name_padding), pad_style),
                Span::styled(" ", pad_style),
                Span::styled(elapsed, elapsed_style),
                Span::styled(" ".repeat(line1_trail), pad_style),
            ]);

            // Line 2: ▌   project name
            let project_display = truncate_with_ellipsis(&project, body_width);
            let project_padding = body_width.saturating_sub(display_width(&project_display));
            let line2 = Line::from(vec![
                Span::styled("▌ ", stripe_bg_style),
                Span::styled(body_indent, pad_style),
                Span::styled(project_display, body_style),
                Span::styled(" ".repeat(project_padding), pad_style),
            ]);

            // Optional: pane_title (task description) when available
            let title = sanitize_pane_title(agent.pane_title.as_deref(), &worktree, &project);

            // Separator at the top (between tiles, not on first item)
            let mut lines = Vec::new();
            if idx > 0 {
                lines.push(Line::from(Span::styled(
                    "─".repeat(sep_width),
                    Style::default().fg(app.palette.border),
                )));
            }

            lines.push(line1);
            lines.push(line2);

            if let Some(title_text) = title {
                let title_display = truncate_with_ellipsis(title_text, body_width);
                let title_padding = body_width.saturating_sub(display_width(&title_display));
                lines.push(Line::from(vec![
                    Span::styled("▌ ", stripe_bg_style),
                    Span::styled(body_indent, pad_style),
                    Span::styled(title_display, body_style),
                    Span::styled(" ".repeat(title_padding), pad_style),
                ]));
            }

            ListItem::new(lines)
        })
        .collect();

    // No highlight_style - background is baked into content lines to avoid highlighting separators
    let list = List::new(items);

    f.render_stateful_widget(list, area, &mut app.list_state);
}

/// Get the status icon string and its style for an agent.
fn status_icon_and_style(
    app: &SidebarApp,
    status: Option<AgentStatus>,
    is_stale: bool,
) -> (String, Style) {
    if is_stale {
        return ("  ".to_string(), Style::default().fg(app.palette.dimmed));
    }
    match status {
        Some(AgentStatus::Working) => {
            let icon = app.status_icons.working.clone().unwrap_or_else(|| {
                let frames: &[&str] = &[
                    "⠃⠀", "⠋⠀", "⠈⠃", "⠀⠋", "⠀⠙", "⠀⠸", "⠀⣰", "⠀⣠", "⢀⡄", "⡄⠂", "⠆⠁", "⠇⠀",
                ];
                frames[app.spinner_frame as usize % frames.len()].to_string()
            });
            (icon, Style::default().fg(app.palette.info))
        }
        Some(AgentStatus::Waiting) => (
            app.status_icons.waiting().to_string(),
            Style::default().fg(app.palette.accent),
        ),
        Some(AgentStatus::Done) => (
            app.status_icons.done().to_string(),
            Style::default().fg(app.palette.success),
        ),
        None => ("  ".to_string(), Style::default().fg(app.palette.dimmed)),
    }
}

/// Clean up a pane title, returning None if it's noise.
fn sanitize_pane_title<'a>(raw: Option<&'a str>, worktree: &str, project: &str) -> Option<&'a str> {
    let title = raw?.trim();
    if title.is_empty() {
        return None;
    }

    // Strip leading status icon characters (braille spinners, symbols like ✳)
    let title = title
        .trim_start_matches(|c: char| {
            // Braille patterns U+2800..U+28FF, common status symbols
            ('\u{2800}'..='\u{28FF}').contains(&c)
                || matches!(c, '✳' | '⠀' | '●' | '○' | '◌' | '✓' | '✗')
        })
        .trim();

    if title.is_empty() {
        return None;
    }

    // Filter out generic "Claude Code" titles (with optional version)
    if title.starts_with("Claude Code") {
        return None;
    }

    // Filter out shell names
    if matches!(title, "zsh" | "bash" | "sh" | "fish") {
        return None;
    }

    // Filter if identical to worktree or project name
    if title == worktree || title == project {
        return None;
    }

    Some(title)
}

/// Format elapsed seconds compactly: "5s", "2m", "1h", "3d"
fn format_compact_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// Get the display width of a string, counting wide chars as 2.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| UnicodeWidthChar::width(c).unwrap_or(1))
        .sum()
}

/// Truncate a string to fit within a given display width (hard cut, no ellipsis).
fn truncate_to_width(s: &str, max_width: usize) -> String {
    let mut width = 0;
    let mut result = String::new();
    for c in s.chars() {
        let w = UnicodeWidthChar::width(c).unwrap_or(1);
        if width + w > max_width {
            break;
        }
        width += w;
        result.push(c);
    }
    result
}

/// Truncate a string to fit within a given display width, adding ellipsis if truncated.
fn truncate_with_ellipsis(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(s) <= max_width {
        return s.to_string();
    }
    if max_width == 1 {
        return "\u{2026}".to_string();
    }

    let mut out = String::new();
    let mut width = 0;
    for c in s.chars() {
        let char_width = UnicodeWidthChar::width(c).unwrap_or(1);
        // Reserve 1 column for the ellipsis character
        if width + char_width + 1 > max_width {
            break;
        }
        out.push(c);
        width += char_width;
    }
    out.push('\u{2026}');
    out
}
