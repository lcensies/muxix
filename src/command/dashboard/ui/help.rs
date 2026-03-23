//! Help overlay rendering.

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Cell, Clear, Paragraph, Row, Table},
};

use super::super::app::{App, DashboardTab, ViewMode};
use super::super::keymap::{Context, help_rows};

/// Determine the current keymap context for help display.
fn get_help_context(app: &App) -> Context {
    match &app.view_mode {
        ViewMode::Dashboard => match app.active_tab {
            DashboardTab::Agents => {
                if app.filter_active {
                    Context::DashboardFilter
                } else if app.input_mode {
                    Context::DashboardInput
                } else {
                    Context::DashboardNormal
                }
            }
            DashboardTab::Worktrees => {
                if app.worktree_filter_active {
                    Context::WorktreeFilter
                } else {
                    Context::WorktreeNormal
                }
            }
        },
        ViewMode::Diff(diff) => {
            if diff.patch_mode {
                if diff.comment_input.is_some() {
                    Context::Comment
                } else {
                    Context::Patch
                }
            } else {
                Context::DiffNormal
            }
        }
    }
}

/// Get the title for the help overlay based on context.
fn context_title(ctx: Context) -> &'static str {
    match ctx {
        Context::DashboardNormal => "Dashboard",
        Context::DashboardInput => "Input Mode",
        Context::DashboardFilter | Context::WorktreeFilter => "Filter",
        Context::WorktreeNormal => "Worktrees",
        Context::DiffNormal => "Diff View",
        Context::Patch => "Patch Mode",
        Context::Comment => "Comment",
    }
}

/// Render the kill confirmation popup.
pub fn render_confirm_kill(f: &mut Frame, app: &App) {
    let palette = &app.palette;

    let height = 3;
    let width = 34;

    let area = f.area();
    let popup_area = Rect {
        x: area.width.saturating_sub(width) / 2,
        y: area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    };

    let block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(palette.help_border));

    let text = Line::from(vec![
        Span::styled(" Kill working agent? ", Style::default().fg(palette.text)),
        Span::styled(
            "y",
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("es / ", Style::default().fg(palette.dimmed)),
        Span::styled(
            "n",
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("o", Style::default().fg(palette.dimmed)),
    ]);

    let paragraph = Paragraph::new(text).block(block);

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
}

/// Render the remove worktree confirmation modal.
pub fn render_confirm_remove(f: &mut Frame, app: &App) {
    let Some(ref plan) = app.pending_remove else {
        return;
    };
    let palette = &app.palette;

    let bold = |s: &str| {
        Span::styled(
            s.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        )
    };
    let dim = |s: &str| Span::styled(s.to_string(), Style::default().fg(palette.dimmed));

    // Build content lines
    let mut lines: Vec<Line> = Vec::new();

    // Title line + spacer
    lines.push(Line::from(vec![Span::styled(
        format!(" Remove {}?", plan.handle),
        Style::default().fg(palette.text),
    )]));
    lines.push(Line::from(""));

    // Warning lines
    if plan.is_dirty {
        lines.push(Line::from(vec![Span::styled(
            " Has uncommitted changes.",
            Style::default().fg(palette.danger),
        )]));
    }
    if plan.is_unmerged {
        lines.push(Line::from(vec![Span::styled(
            " Has unmerged commits.",
            Style::default().fg(palette.dimmed),
        )]));
    }

    // Branch outcome line
    if plan.keep_branch {
        lines.push(Line::from(vec![Span::styled(
            " Branch will be kept.",
            Style::default().fg(palette.dimmed),
        )]));
    } else {
        lines.push(Line::from(vec![Span::styled(
            " Branch will be deleted.",
            Style::default().fg(palette.dimmed),
        )]));
    }

    // Empty line before actions
    lines.push(Line::from(""));

    // Action line (context-dependent)
    let action_line = if plan.is_dirty && !plan.force_armed {
        // Dirty: must press f to arm force
        Line::from(vec![
            Span::raw(" "),
            bold("f"),
            dim(" force  "),
            bold("n"),
            dim(" cancel  "),
            bold("k"),
            if plan.keep_branch {
                dim(" delete branch")
            } else {
                dim(" keep branch")
            },
        ])
    } else if plan.is_dirty && plan.force_armed {
        // Dirty + force armed: y now available
        Line::from(vec![
            Span::raw(" "),
            bold("y"),
            dim(" confirm force  "),
            bold("n"),
            dim(" cancel  "),
            bold("k"),
            if plan.keep_branch {
                dim(" delete branch")
            } else {
                dim(" keep branch")
            },
        ])
    } else {
        // Clean or unmerged: y available
        Line::from(vec![
            Span::raw(" "),
            bold("y"),
            dim(" remove  "),
            bold("n"),
            dim(" cancel  "),
            bold("k"),
            if plan.keep_branch {
                dim(" delete branch")
            } else {
                dim(" keep branch")
            },
        ])
    };
    lines.push(action_line);

    // Calculate dimensions
    let height = lines.len() as u16 + 2; // +2 for borders
    let width = 44;

    let area = f.area();
    let popup_area = Rect {
        x: area.width.saturating_sub(width) / 2,
        y: area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    };

    let block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(palette.help_border));

    let paragraph = Paragraph::new(Text::from(lines)).block(block);

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
}

/// Render the help overlay.
pub fn render_help(f: &mut Frame, app: &App) {
    let ctx = get_help_context(app);
    let title = context_title(ctx);
    let keybindings = help_rows(ctx);

    // Calculate dimensions based on content
    let row_count = keybindings.len() as u16;
    let height = row_count + 5; // +5 for borders, padding, and empty line at top
    let width = 44;

    // Center the popup
    let area = f.area();
    let popup_area = Rect {
        x: area.width.saturating_sub(width) / 2,
        y: area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    };

    let palette = &app.palette;

    // Create styled block with rounded corners
    let block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(palette.help_border))
        .title(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                title,
                Style::default()
                    .fg(palette.header)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
        ]))
        .title_bottom(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled("any key", Style::default().fg(palette.dimmed)),
            Span::styled(" to close ", Style::default().fg(palette.help_muted)),
        ]));

    // Build styled rows with empty line at top for padding
    let mut rows: Vec<Row> = vec![Row::new(vec![Cell::from(""), Cell::from("")])];
    rows.extend(keybindings.into_iter().map(|(key, desc)| {
        Row::new(vec![
            Cell::from(Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(
                    format!("{:>8}", key),
                    Style::default()
                        .fg(palette.dimmed)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            Cell::from(Line::from(vec![
                Span::styled(" · ", Style::default().fg(palette.help_muted)),
                Span::styled(desc, Style::default().fg(palette.text)),
            ])),
        ])
    }));

    let table = Table::new(rows, [Constraint::Length(10), Constraint::Min(25)])
        .block(block)
        .column_spacing(0);

    f.render_widget(Clear, popup_area);
    f.render_widget(table, popup_area);
}

/// Render the sweep cleanup modal.
pub fn render_sweep(f: &mut Frame, app: &App) {
    let Some(ref sweep) = app.pending_sweep else {
        return;
    };
    let palette = &app.palette;

    let bold = |s: &str| {
        Span::styled(
            s.to_string(),
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD),
        )
    };
    let dim = |s: &str| Span::styled(s.to_string(), Style::default().fg(palette.dimmed));

    // Empty state
    if sweep.candidates.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::from(vec![Span::styled(
                " Nothing to sweep.",
                Style::default().fg(palette.dimmed),
            )]),
            Line::from(""),
        ];

        let height = lines.len() as u16 + 2;
        let width = 30;
        let area = f.area();
        let popup_area = Rect {
            x: area.width.saturating_sub(width) / 2,
            y: area.height.saturating_sub(height) / 2,
            width: width.min(area.width),
            height: height.min(area.height),
        };

        let block = Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .border_style(Style::default().fg(palette.help_border))
            .title(Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(
                    "Sweep",
                    Style::default()
                        .fg(palette.header)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(" ", Style::default()),
            ]));

        let paragraph = Paragraph::new(Text::from(lines)).block(block);
        f.render_widget(Clear, popup_area);
        f.render_widget(paragraph, popup_area);
        return;
    }

    let selected_count = sweep.candidates.iter().filter(|c| c.selected).count();

    // Build content lines
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(""));

    for (i, candidate) in sweep.candidates.iter().enumerate() {
        let cursor = if i == sweep.cursor { "> " } else { "  " };
        let cursor_style = Style::default().fg(palette.text);

        if candidate.is_dirty {
            // Dirty: greyed out, not selectable
            lines.push(Line::from(vec![
                Span::styled(cursor, cursor_style),
                dim(&format!(
                    "--- {} ({}, dirty)",
                    candidate.handle,
                    candidate.reason.label()
                )),
            ]));
        } else {
            let checkbox = if candidate.selected { "[x]" } else { "[ ]" };
            let style = Style::default().fg(palette.text);
            lines.push(Line::from(vec![
                Span::styled(cursor, cursor_style),
                Span::styled(format!("{} {} ", checkbox, candidate.handle), style),
                dim(&format!("({})", candidate.reason.label())),
            ]));
        }
    }

    lines.push(Line::from(""));

    // Action line
    let remove_label = if selected_count > 0 {
        format!(" remove ({})", selected_count)
    } else {
        " remove".to_string()
    };
    lines.push(Line::from(vec![
        Span::raw(" "),
        bold("Space"),
        dim(" toggle  "),
        bold("Enter"),
        dim(&remove_label),
        dim("  "),
        bold("Esc"),
        dim(" cancel"),
    ]));

    // Calculate dimensions
    let height = lines.len() as u16 + 2; // +2 for borders
    let content_width = sweep
        .candidates
        .iter()
        .map(|c| {
            // cursor + checkbox/dash + handle + reason
            2 + 4 + c.handle.len() + c.reason.label().len() + 10
        })
        .max()
        .unwrap_or(30);
    let width = (content_width as u16 + 4).max(44); // +4 for border+padding

    let area = f.area();
    let popup_area = Rect {
        x: area.width.saturating_sub(width) / 2,
        y: area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    };

    let block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(palette.help_border))
        .title(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                "Sweep",
                Style::default()
                    .fg(palette.header)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
        ]));

    let paragraph = Paragraph::new(Text::from(lines)).block(block);

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
}
