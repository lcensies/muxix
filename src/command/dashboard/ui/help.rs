//! Help overlay rendering.

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
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

/// Render the delete worktree confirmation popup.
pub fn render_confirm_delete_worktree(f: &mut Frame, app: &App) {
    let palette = &app.palette;

    let height = 3;
    let width = 46;

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
        Span::styled(
            " Has uncommitted/unmerged changes. ",
            Style::default().fg(palette.text),
        ),
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
