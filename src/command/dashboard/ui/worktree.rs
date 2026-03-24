//! Worktree table rendering for the dashboard worktree view.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Cell, Paragraph, Row, Table},
};

use super::super::agent;
use super::super::app::App;
use super::super::spinner::SPINNER_FRAMES;
use super::format::{format_git_status, format_pr_status};

/// Render the worktree table in the given area.
pub fn render_worktree_table(f: &mut Frame, app: &mut App, area: Rect) {
    // Don't render headers for an empty table - avoids a visual blink
    // as column widths jump when data arrives on the next frame
    if app.worktrees.is_empty() {
        return;
    }

    let show_check_counts = app.config.dashboard.show_check_counts();

    // Only show PR column when at least one worktree has a PR
    let show_pr_column = app.worktrees.iter().any(|w| w.pr_info.is_some());

    // Check if git data is being refreshed
    let is_git_fetching = app
        .is_git_fetching
        .load(std::sync::atomic::Ordering::Relaxed);

    // Build Git header with spinner when fetching
    let git_header = if is_git_fetching {
        let spinner = SPINNER_FRAMES[app.spinner_frame as usize % SPINNER_FRAMES.len()];
        Line::from(vec![
            Span::styled("Git ", Style::default().fg(app.palette.header).bold()),
            Span::styled(spinner.to_string(), Style::default().fg(app.palette.dimmed)),
        ])
    } else {
        Line::from(Span::styled(
            "Git",
            Style::default().fg(app.palette.header).bold(),
        ))
    };

    let header_style = Style::default().fg(app.palette.header).bold();
    let mut header_cells = vec![
        Cell::from("#").style(header_style),
        Cell::from("Project").style(header_style),
        Cell::from("Worktree").style(header_style),
        Cell::from(git_header),
    ];
    if show_pr_column {
        header_cells.push(Cell::from("PR").style(header_style));
    }
    header_cells.extend([
        Cell::from("Mux").style(header_style),
        Cell::from("Age").style(header_style),
    ]);
    header_cells.push(Cell::from("Agent").style(header_style));
    let header = Row::new(header_cells).height(1);

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

            // Main worktree: show branch name (handle is just the repo dir name)
            // Other worktrees: show branch inline when it differs from the handle
            let worktree_display = if wt.is_main {
                wt.branch.clone()
            } else if wt.branch != wt.handle {
                format!("{} \u{2192}{}", wt.handle, wt.branch)
            } else {
                wt.handle.clone()
            };

            // Git status
            let git_status = app.git_statuses.get(&wt.path);
            let git_spans = format_git_status(git_status, app.spinner_frame, &app.palette);

            // PR status (only computed if column is shown)
            let pr_spans = if show_pr_column {
                Some(format_pr_status(
                    wt.pr_info.as_ref(),
                    show_check_counts,
                    &app.palette,
                ))
            } else {
                None
            };

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
                        format!("{} {} ", icon, spinner),
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

            let is_current = app.current_worktree.as_ref().is_some_and(|cwd| {
                if let (Ok(cwd_canonical), Ok(wt_canonical)) =
                    (cwd.canonicalize(), wt.path.canonicalize())
                {
                    cwd_canonical == wt_canonical
                } else {
                    wt.path == *cwd
                }
            });

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let age = wt
                .created_at
                .map(|ts| agent::format_age(now.saturating_sub(ts)));

            (
                jump_key,
                project,
                worktree_display,
                wt.is_main,
                is_current,
                git_spans,
                pr_spans,
                agent_spans,
                wt.has_mux_window,
                age,
            )
        })
        .collect();

    // Calculate dynamic column widths
    let max_project_width = row_data
        .iter()
        .map(|(_, p, _, _, _, _, _, _, _, _)| p.len())
        .max()
        .unwrap_or(5)
        .clamp(5, 20)
        + 2;

    let max_worktree_width = row_data
        .iter()
        .map(|(_, _, w, _, _, _, _, _, _, _)| w.len())
        .max()
        .unwrap_or(8)
        .max(8)
        + 1;

    let max_git_width = row_data
        .iter()
        .map(|(_, _, _, _, _, git, _, _, _, _)| {
            git.iter()
                .map(|(text, _)| text.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(4)
        .clamp(4, 30)
        + 1;

    let max_pr_width = if show_pr_column {
        row_data
            .iter()
            .filter_map(|(_, _, _, _, _, _, pr, _, _, _)| pr.as_ref())
            .map(|spans| {
                spans
                    .iter()
                    .map(|(text, _)| text.chars().count())
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(4)
            .clamp(4, 16)
            + 1
    } else {
        0
    };

    let rows: Vec<Row> = row_data
        .into_iter()
        .map(
            |(
                jump_key,
                project,
                worktree_display,
                is_main,
                is_current,
                git_spans,
                pr_spans,
                agent_spans,
                has_mux_window,
                age,
            )| {
                let worktree_style = if is_current {
                    Style::default().fg(app.palette.current_worktree_fg)
                } else if is_main {
                    Style::default().fg(app.palette.dimmed)
                } else {
                    Style::default()
                };

                let git_line = Line::from(
                    git_spans
                        .into_iter()
                        .map(|(text, style)| Span::styled(text, style))
                        .collect::<Vec<_>>(),
                );

                let mux_cell = if has_mux_window {
                    Cell::from("\u{25cf}").style(Style::default().fg(app.palette.success))
                } else {
                    Cell::from("-").style(Style::default().fg(app.palette.dimmed))
                };

                let agent_line = Line::from(
                    agent_spans
                        .into_iter()
                        .map(|(text, style)| Span::styled(text, style))
                        .collect::<Vec<_>>(),
                );

                let age_cell = Cell::from(age.unwrap_or_default())
                    .style(Style::default().fg(app.palette.dimmed));

                let mut cells = vec![
                    Cell::from(jump_key).style(Style::default().fg(app.palette.keycap)),
                    Cell::from(project),
                    Cell::from(worktree_display).style(worktree_style),
                    Cell::from(git_line),
                ];

                if let Some(pr_spans) = pr_spans {
                    let pr_line = Line::from(
                        pr_spans
                            .into_iter()
                            .map(|(text, style)| Span::styled(text, style))
                            .collect::<Vec<_>>(),
                    );
                    cells.push(Cell::from(pr_line));
                }

                cells.extend([mux_cell, age_cell]);
                cells.push(Cell::from(agent_line));

                let row = Row::new(cells);
                if is_current {
                    row.style(Style::default().bg(app.palette.current_row_bg))
                } else {
                    row
                }
            },
        )
        .collect();

    let mut constraints = vec![
        Constraint::Length(2),                         // #
        Constraint::Length(max_project_width as u16),  // Project
        Constraint::Length(max_worktree_width as u16), // Worktree (+ branch when different)
        Constraint::Length(max_git_width as u16),      // Git
    ];
    if show_pr_column {
        constraints.push(Constraint::Length(max_pr_width as u16));
    }
    constraints.extend([
        Constraint::Length(4), // Mux
        Constraint::Length(4), // Age
    ]);
    constraints.push(Constraint::Fill(1)); // Agent

    let table = Table::new(rows, constraints)
        .header(header)
        .block(Block::default())
        .row_highlight_style(Style::default().bg(app.palette.highlight_row_bg))
        .highlight_symbol("> ");

    f.render_stateful_widget(table, area, &mut app.worktree_table_state);
}

/// Render the worktree preview: info panel (left) + styled git log (right).
pub fn render_worktree_preview(f: &mut Frame, app: &mut App, area: Rect) {
    let selected_worktree = app
        .worktree_table_state
        .selected()
        .and_then(|idx| app.worktrees.get(idx));

    // Split preview area into info panel (left) and git log (right)
    let chunks = Layout::horizontal([
        Constraint::Length(40), // Info panel: fixed width
        Constraint::Fill(1),    // Git log: remaining space
    ])
    .split(area);

    render_info_panel(f, app, chunks[0], selected_worktree);
    render_git_log(f, app, chunks[1], selected_worktree);
}

/// Render the info panel showing worktree metadata.
fn render_info_panel(
    f: &mut Frame,
    app: &App,
    area: Rect,
    worktree: Option<&crate::workflow::types::WorktreeInfo>,
) {
    let title_style = Style::default()
        .fg(app.palette.header)
        .add_modifier(Modifier::BOLD);
    let border_style = Style::default().fg(app.palette.border);
    let label_style = Style::default().fg(app.palette.dimmed);
    let text_style = Style::default().fg(app.palette.text);

    let title = if let Some(wt) = worktree {
        format!(" {} ", wt.handle)
    } else {
        " Info ".to_string()
    };

    let block = Block::bordered()
        .title(title)
        .title_style(title_style)
        .border_style(border_style);

    let Some(wt) = worktree else {
        let paragraph = Paragraph::new(Text::raw("(no worktree selected)")).block(block);
        f.render_widget(paragraph, area);
        return;
    };

    let mut lines: Vec<Line> = Vec::new();

    // Branch
    lines.push(Line::from(vec![
        Span::styled("Branch  ", label_style),
        Span::styled(&wt.branch, text_style),
    ]));

    // Git status details (base branch, ahead/behind, diff stats)
    let git_status = app.git_statuses.get(&wt.path);
    if let Some(status) = git_status {
        // Base branch + ahead/behind
        let mut base_spans = vec![Span::styled("Base    ", label_style)];
        if !status.base_branch.is_empty() {
            base_spans.push(Span::styled(&status.base_branch, text_style));
        } else {
            base_spans.push(Span::styled("main", text_style));
        }
        if status.ahead > 0 || status.behind > 0 {
            base_spans.push(Span::styled(" (", label_style));
            if status.ahead > 0 {
                base_spans.push(Span::styled(
                    format!("\u{2191}{}", status.ahead),
                    Style::default().fg(app.palette.info),
                ));
            }
            if status.ahead > 0 && status.behind > 0 {
                base_spans.push(Span::styled(" ", label_style));
            }
            if status.behind > 0 {
                base_spans.push(Span::styled(
                    format!("\u{2193}{}", status.behind),
                    Style::default().fg(app.palette.accent),
                ));
            }
            base_spans.push(Span::styled(")", label_style));
        }
        lines.push(Line::from(base_spans));

        // Committed diff stats
        if status.lines_added > 0 || status.lines_removed > 0 {
            let mut diff_spans = vec![Span::styled("Diff    ", label_style)];
            if status.lines_added > 0 {
                diff_spans.push(Span::styled(
                    format!("+{}", status.lines_added),
                    Style::default().fg(app.palette.success),
                ));
            }
            if status.lines_added > 0 && status.lines_removed > 0 {
                diff_spans.push(Span::styled(" ", text_style));
            }
            if status.lines_removed > 0 {
                diff_spans.push(Span::styled(
                    format!("-{}", status.lines_removed),
                    Style::default().fg(app.palette.danger),
                ));
            }
            diff_spans.push(Span::styled(" committed", label_style));
            lines.push(Line::from(diff_spans));
        }

        // Uncommitted changes
        if status.uncommitted_added > 0 || status.uncommitted_removed > 0 {
            let mut uc_spans = vec![Span::styled("        ", label_style)];
            if status.uncommitted_added > 0 {
                uc_spans.push(Span::styled(
                    format!("+{}", status.uncommitted_added),
                    Style::default().fg(app.palette.success),
                ));
            }
            if status.uncommitted_added > 0 && status.uncommitted_removed > 0 {
                uc_spans.push(Span::styled(" ", text_style));
            }
            if status.uncommitted_removed > 0 {
                uc_spans.push(Span::styled(
                    format!("-{}", status.uncommitted_removed),
                    Style::default().fg(app.palette.danger),
                ));
            }
            uc_spans.push(Span::styled(" uncommitted", label_style));
            lines.push(Line::from(uc_spans));
        }

        // Conflict indicator
        if status.has_conflict {
            lines.push(Line::from(vec![
                Span::styled("        ", label_style),
                Span::styled(
                    "conflict with base",
                    Style::default().fg(app.palette.danger),
                ),
            ]));
        }
    }

    // PR info
    if let Some(ref pr) = wt.pr_info {
        let pr_icons = crate::nerdfont::pr_icons();
        let (icon, color) = if pr.is_draft {
            (pr_icons.draft, app.palette.dimmed)
        } else {
            match pr.state.as_str() {
                "OPEN" => (pr_icons.open, app.palette.success),
                "MERGED" => (pr_icons.merged, app.palette.accent),
                "CLOSED" => (pr_icons.closed, app.palette.danger),
                _ => ("?", app.palette.dimmed),
            }
        };
        let mut pr_spans = vec![
            Span::styled("PR      ", label_style),
            Span::styled(format!("#{} ", pr.number), Style::default().fg(color)),
            Span::styled(icon, Style::default().fg(color)),
        ];
        // Check status
        if let Some(ref checks) = pr.checks {
            use crate::github::CheckState;
            let check_icons = crate::nerdfont::check_icons();
            let (check_icon, check_color) = match checks {
                CheckState::Success => (check_icons.success, app.palette.success),
                CheckState::Failure { .. } => (check_icons.failure, app.palette.danger),
                CheckState::Pending { .. } => (check_icons.pending, app.palette.accent),
            };
            pr_spans.push(Span::styled(" ", text_style));
            pr_spans.push(Span::styled(
                check_icon.to_string(),
                Style::default().fg(check_color),
            ));
        }
        lines.push(Line::from(pr_spans));

        // PR title (truncated to fit)
        let inner_width = area.width.saturating_sub(2) as usize; // border
        let title_max = inner_width.saturating_sub(8); // label width
        let truncated_title = if pr.title.len() > title_max {
            format!("{}...", &pr.title[..title_max.saturating_sub(3)])
        } else {
            pr.title.clone()
        };
        lines.push(Line::from(vec![
            Span::styled("        ", label_style),
            Span::styled(truncated_title, Style::default().fg(color)),
        ]));

        // Check detail: failing check name or pending elapsed time
        let detail_spans = super::format::format_pr_details(pr, &app.palette);
        if !detail_spans.is_empty() {
            let mut line_spans = vec![Span::styled("        ", label_style)];
            line_spans.extend(detail_spans);
            lines.push(Line::from(line_spans));
        }
    }

    // Agent status
    if let Some(ref summary) = wt.agent_status {
        use crate::multiplexer::AgentStatus;
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

        let mut agent_spans = vec![Span::styled("Agent   ", label_style)];
        if working > 0 {
            let icon = app.config.status_icons.working();
            let spinner = SPINNER_FRAMES[app.spinner_frame as usize % SPINNER_FRAMES.len()];
            agent_spans.push(Span::styled(
                format!("{} {}", icon, spinner),
                Style::default().fg(app.palette.info),
            ));
        }
        if waiting > 0 {
            if working > 0 {
                agent_spans.push(Span::styled(" ", text_style));
            }
            let icon = app.config.status_icons.waiting();
            agent_spans.push(Span::styled(
                icon.to_string(),
                Style::default().fg(app.palette.accent),
            ));
        }
        if done > 0 {
            if working > 0 || waiting > 0 {
                agent_spans.push(Span::styled(" ", text_style));
            }
            let icon = app.config.status_icons.done();
            agent_spans.push(Span::styled(
                icon.to_string(),
                Style::default().fg(app.palette.success),
            ));
        }
        lines.push(Line::from(agent_spans));
    }

    // Mux window
    let mux_spans = vec![
        Span::styled("Mux     ", label_style),
        if wt.has_mux_window {
            Span::styled("\u{25cf} active", Style::default().fg(app.palette.success))
        } else {
            Span::styled("- none", Style::default().fg(app.palette.dimmed))
        },
    ];
    lines.push(Line::from(mux_spans));

    let paragraph = Paragraph::new(Text::from(lines)).block(block);
    f.render_widget(paragraph, area);
}

/// Render the styled git log panel.
fn render_git_log(
    f: &mut Frame,
    app: &App,
    area: Rect,
    worktree: Option<&crate::workflow::types::WorktreeInfo>,
) {
    let title_style = Style::default()
        .fg(app.palette.header)
        .add_modifier(Modifier::BOLD);
    let border_style = Style::default().fg(app.palette.border);

    let block = Block::bordered()
        .title(" Git Log ")
        .title_style(title_style)
        .border_style(border_style);

    let text = match (&app.worktree_preview, worktree) {
        (Some(log), Some(_)) if !log.trim().is_empty() => {
            let hash_style = Style::default().fg(app.palette.accent);
            let date_style = Style::default().fg(app.palette.dimmed);
            let msg_style = Style::default().fg(app.palette.text);

            let lines: Vec<Line> = log
                .lines()
                .map(|line| {
                    let parts: Vec<&str> = line.splitn(3, '\t').collect();
                    if parts.len() == 3 {
                        Line::from(vec![
                            Span::styled(parts[0], hash_style),
                            Span::styled("  ", date_style),
                            Span::styled(parts[1], date_style),
                            Span::styled("  ", msg_style),
                            Span::styled(parts[2], msg_style),
                        ])
                    } else {
                        // Fallback for lines that don't match format
                        Line::styled(line, msg_style)
                    }
                })
                .collect();
            Text::from(lines)
        }
        (None, Some(_)) => Text::raw(""),
        (Some(_), Some(_)) => Text::raw("(no commits)"),
        (_, None) => Text::raw(""),
    };

    let paragraph = Paragraph::new(text).block(block);
    f.render_widget(paragraph, area);
}
