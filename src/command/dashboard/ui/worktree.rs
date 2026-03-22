//! Worktree table rendering for the dashboard worktree view.

use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Cell, Paragraph, Row, Table},
};

use super::super::agent;
use super::super::app::App;
use super::super::spinner::SPINNER_FRAMES;
use super::format::format_pr_status;

/// Render the worktree table in the given area.
pub fn render_worktree_table(f: &mut Frame, app: &mut App, area: Rect) {
    let show_check_counts = app.config.dashboard.show_check_counts();

    let header_style = Style::default().fg(app.palette.header).bold();
    let header = Row::new(vec![
        Cell::from("#").style(header_style),
        Cell::from("Project").style(header_style),
        Cell::from("Worktree").style(header_style),
        Cell::from("Branch").style(header_style),
        Cell::from("PR").style(header_style),
        Cell::from("Agent").style(header_style),
    ])
    .height(1);

    // Pre-compute row data
    let row_data: Vec<_> = app
        .worktrees
        .iter()
        .enumerate()
        .map(|(idx, wt)| {
            let jump_key = if idx < 9 {
                format!("{}", idx + 1)
            } else {
                String::new()
            };

            let project = agent::extract_project_name(&wt.path);
            let handle = wt.handle.clone();
            let branch = wt.branch.clone();

            // PR status
            let pr_spans = format_pr_status(wt.pr_info.as_ref(), show_check_counts, &app.palette);

            // Agent status summary
            let agent_spans = if let Some(ref summary) = wt.agent_status {
                use crate::multiplexer::AgentStatus;
                let mut parts: Vec<(String, Style)> = Vec::new();
                let working = summary
                    .statuses
                    .iter()
                    .filter(|s| **s == AgentStatus::Working)
                    .count();
                let waiting = summary
                    .statuses
                    .iter()
                    .filter(|s| **s == AgentStatus::Waiting)
                    .count();
                let done = summary
                    .statuses
                    .iter()
                    .filter(|s| **s == AgentStatus::Done)
                    .count();

                if working > 0 {
                    let icon = app.config.status_icons.working();
                    let spinner = SPINNER_FRAMES[app.spinner_frame as usize % SPINNER_FRAMES.len()];
                    parts.push((
                        format!("{}{} ", icon, spinner),
                        Style::default().fg(app.palette.info),
                    ));
                }
                if waiting > 0 {
                    let icon = app.config.status_icons.waiting();
                    parts.push((
                        format!("{} ", icon),
                        Style::default().fg(app.palette.accent),
                    ));
                }
                if done > 0 {
                    let icon = app.config.status_icons.done();
                    parts.push((
                        format!("{} ", icon),
                        Style::default().fg(app.palette.success),
                    ));
                }
                if parts.is_empty() {
                    parts.push(("-".to_string(), Style::default().fg(app.palette.dimmed)));
                }
                parts
            } else {
                vec![("-".to_string(), Style::default().fg(app.palette.dimmed))]
            };

            (
                jump_key,
                project,
                handle,
                branch,
                wt.is_main,
                pr_spans,
                agent_spans,
            )
        })
        .collect();

    // Calculate dynamic column widths
    let max_project_width = row_data
        .iter()
        .map(|(_, p, _, _, _, _, _)| p.len())
        .max()
        .unwrap_or(5)
        .clamp(5, 20)
        + 2;

    let max_handle_width = row_data
        .iter()
        .map(|(_, _, h, _, _, _, _)| h.len())
        .max()
        .unwrap_or(8)
        .max(8)
        + 1;

    let max_branch_width = row_data
        .iter()
        .map(|(_, _, _, b, _, _, _)| b.len())
        .max()
        .unwrap_or(6)
        .clamp(6, 30)
        + 1;

    let max_pr_width = row_data
        .iter()
        .map(|(_, _, _, _, _, pr, _)| {
            pr.iter()
                .map(|(text, _)| text.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(4)
        .clamp(4, 16)
        + 1;

    let rows: Vec<Row> = row_data
        .into_iter()
        .map(
            |(jump_key, project, handle, branch, is_main, pr_spans, agent_spans)| {
                let worktree_style = if is_main {
                    Style::default().fg(app.palette.dimmed)
                } else {
                    Style::default()
                };

                let pr_line = Line::from(
                    pr_spans
                        .into_iter()
                        .map(|(text, style)| Span::styled(text, style))
                        .collect::<Vec<_>>(),
                );

                let agent_line = Line::from(
                    agent_spans
                        .into_iter()
                        .map(|(text, style)| Span::styled(text, style))
                        .collect::<Vec<_>>(),
                );

                Row::new(vec![
                    Cell::from(jump_key).style(Style::default().fg(app.palette.keycap)),
                    Cell::from(project),
                    Cell::from(handle).style(worktree_style),
                    Cell::from(branch),
                    Cell::from(pr_line),
                    Cell::from(agent_line),
                ])
            },
        )
        .collect();

    let constraints = vec![
        Constraint::Length(2),                        // #
        Constraint::Length(max_project_width as u16), // Project
        Constraint::Length(max_handle_width as u16),  // Worktree
        Constraint::Length(max_branch_width as u16),  // Branch
        Constraint::Length(max_pr_width as u16),      // PR
        Constraint::Fill(1),                          // Agent
    ];

    let table = Table::new(rows, constraints)
        .header(header)
        .block(Block::default())
        .row_highlight_style(Style::default().bg(app.palette.highlight_row_bg))
        .highlight_symbol("> ");

    f.render_stateful_widget(table, area, &mut app.worktree_table_state);
}

/// Render the worktree preview (git log output).
pub fn render_worktree_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let selected_worktree = app
        .worktree_table_state
        .selected()
        .and_then(|idx| app.worktrees.get(idx));

    let (title, title_style) = if let Some(wt) = selected_worktree {
        (
            format!(" Preview: {} ", wt.handle),
            Style::default()
                .fg(app.palette.header)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            " Preview ".to_string(),
            Style::default()
                .fg(app.palette.header)
                .add_modifier(Modifier::BOLD),
        )
    };

    let block = Block::bordered()
        .title(title)
        .title_style(title_style)
        .border_style(Style::default().fg(app.palette.border));

    let text = match (&app.worktree_preview, selected_worktree) {
        (Some(log), Some(_)) if !log.trim().is_empty() => Text::raw(log.as_str()),
        (None, Some(_)) => Text::raw("(loading...)"),
        (Some(_), Some(_)) => Text::raw("(no commits)"),
        (_, None) => Text::raw("(no worktree selected)"),
    };

    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, area);
}
