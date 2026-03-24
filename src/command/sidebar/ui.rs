//! Rendering for the sidebar TUI.

use ratatui::Frame;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Padding};
use std::time::{SystemTime, UNIX_EPOCH};
use unicode_width::UnicodeWidthChar;

use crate::multiplexer::AgentStatus;

use super::app::SidebarApp;

/// Render the sidebar UI.
pub fn render_sidebar(f: &mut Frame, app: &mut SidebarApp) {
    let area = f.area();

    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(app.palette.border))
        .padding(Padding::new(1, 1, 0, 0));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.agents.is_empty() {
        let empty_line = Line::from(Span::styled(
            "No agents",
            Style::default().fg(app.palette.dimmed),
        ));
        let list = List::new(vec![ListItem::new(empty_line)]);
        f.render_widget(list, inner);
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
            let (icon, icon_style) = if is_stale {
                ("  ".to_string(), Style::default().fg(app.palette.dimmed))
            } else {
                match agent.status {
                    Some(AgentStatus::Working) => {
                        let icon = app.status_icons.working.clone().unwrap_or_else(|| {
                            // 3-dot comet orbiting the 4x4 braille grid
                            let frames: &[&str] = &[
                                "⠃⠀", "⠋⠀", "⠈⠃", "⠀⠋", "⠀⠙", "⠀⠸", "⠀⣰", "⠀⣠", "⢀⡄", "⡄⠂", "⠆⠁",
                                "⠇⠀",
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
            };

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
            let name_width = (inner.width as usize).saturating_sub(reserved);

            let display_name = truncate_to_width(&worktree_name, name_width);
            let padding = name_width.saturating_sub(display_width(&display_name));

            let name_style = if is_stale {
                Style::default()
                    .fg(app.palette.dimmed)
                    .add_modifier(Modifier::DIM)
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

    f.render_stateful_widget(list, inner, &mut app.list_state);
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

/// Truncate a string to fit within a given display width.
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
