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
                " No merged or gone worktrees found.",
                Style::default().fg(palette.dimmed),
            )]),
            Line::from(""),
        ];

        let height = lines.len() as u16 + 2;
        let width = 38;
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
                    "[ ] {} ({}, dirty)",
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
            // cursor + checkbox + handle + reason
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

/// Render the base branch picker modal.
pub fn render_base_picker(f: &mut Frame, app: &App) {
    let Some(ref picker) = app.pending_base_picker else {
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

    let filtered = picker.filtered();

    let content_width = picker
        .branches
        .iter()
        .map(|b| 2 + b.len())
        .max()
        .unwrap_or(20);
    let width = (content_width as u16 + 4).clamp(44, 60);
    // Fixed height: ~40% of terminal, matching add-worktree modal
    let area = f.area();
    let height = (area.height * 2 / 5).clamp(10, 25);
    // 1 filter + 1 blank + visible items + 1 blank + 1 footer + 2 borders
    let max_visible: usize = height.saturating_sub(6) as usize;

    let mut lines: Vec<Line> = Vec::new();

    // Filter input line (always present to keep layout stable)
    if picker.filter.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" /", Style::default().fg(palette.dimmed)),
            Span::styled("_", Style::default().fg(palette.dimmed)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" /", Style::default().fg(palette.dimmed)),
            Span::styled(picker.filter.clone(), Style::default().fg(palette.text)),
            Span::styled("_", Style::default().fg(palette.text)),
        ]));
    }

    lines.push(Line::from(""));

    if filtered.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            " No matching branches.",
            Style::default().fg(palette.dimmed),
        )]));
        // Fill remaining slots so height stays fixed
        for _ in 1..max_visible {
            lines.push(Line::from(""));
        }
    } else {
        // Compute a window of items around the cursor
        let total = filtered.len();
        let start = if total <= max_visible || picker.cursor < max_visible / 2 {
            0
        } else if picker.cursor + max_visible / 2 >= total {
            total.saturating_sub(max_visible)
        } else {
            picker.cursor - max_visible / 2
        };
        let end = (start + max_visible).min(total);

        for (fi, &idx) in filtered.iter().enumerate().take(end).skip(start) {
            let branch = &picker.branches[idx];
            let cursor = if fi == picker.cursor { "> " } else { "  " };

            let is_current = picker.current_base.as_ref().is_some_and(|b| b == branch);

            let name_style = if is_current {
                Style::default().fg(palette.accent)
            } else {
                Style::default().fg(palette.text)
            };

            lines.push(Line::from(vec![
                Span::styled(cursor, Style::default().fg(palette.text)),
                Span::styled(branch.clone(), name_style),
            ]));
        }

        // Fill remaining slots so height stays fixed
        for _ in (end - start)..max_visible {
            lines.push(Line::from(""));
        }
    }

    lines.push(Line::from(""));

    // Footer
    lines.push(Line::from(vec![
        Span::raw(" "),
        bold("Enter"),
        dim(" set base  "),
        bold("Esc"),
        dim(" cancel"),
    ]));

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
                "Set Base Branch",
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

/// Render the project picker modal.
pub fn render_project_picker(f: &mut Frame, app: &App) {
    let Some(ref picker) = app.pending_project_picker else {
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

    let filtered = picker.filtered();

    let mut lines: Vec<Line> = Vec::new();

    // Filter input line (shown when typing)
    if !picker.filter.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" /", Style::default().fg(palette.dimmed)),
            Span::styled(picker.filter.clone(), Style::default().fg(palette.text)),
            Span::styled("_", Style::default().fg(palette.text)),
        ]));
    }

    lines.push(Line::from(""));

    if filtered.is_empty() {
        lines.push(Line::from(vec![Span::styled(
            " No matching projects.",
            Style::default().fg(palette.dimmed),
        )]));
    } else {
        for (fi, &idx) in filtered.iter().enumerate() {
            let project = &picker.projects[idx];
            let cursor = if fi == picker.cursor { "> " } else { "  " };

            let is_current = picker
                .current_name
                .as_ref()
                .is_some_and(|n| *n == project.name);

            let name_style = if is_current {
                Style::default().fg(palette.accent)
            } else {
                Style::default().fg(palette.text)
            };

            lines.push(Line::from(vec![
                Span::styled(cursor, Style::default().fg(palette.text)),
                Span::styled(project.name.clone(), name_style),
            ]));
        }
    }

    lines.push(Line::from(""));

    // Footer
    lines.push(Line::from(vec![
        Span::raw(" "),
        bold("Enter"),
        dim(" switch  "),
        bold("Esc"),
        dim(" cancel"),
    ]));

    // Calculate dimensions
    let height = lines.len() as u16 + 2;
    let content_width = picker
        .projects
        .iter()
        .map(|p| 2 + p.name.len())
        .max()
        .unwrap_or(20);
    let width = (content_width as u16 + 4).clamp(36, 60);

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
                "Switch Project",
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

/// Render the add-worktree modal.
pub fn render_add_worktree(f: &mut Frame, app: &App) {
    use super::super::app::{AddWorktreeMode, PrListState};

    let Some(ref state) = app.pending_add_worktree else {
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

    let is_pr_mode = state.mode == AddWorktreeMode::Pr;

    let area = f.area();
    let width = (area.width * 3 / 5).clamp(44, 80);

    let area = f.area();
    let height = (area.height * 2 / 5).clamp(10, 25);
    // overhead: filter + blank + action_row + blank + footer + blank_after_footer + borders(2)
    let has_action_row = !is_pr_mode && !state.filter.trim().is_empty();
    let overhead: u16 = 7 + if has_action_row { 1 } else { 0 };
    let max_visible: usize = height.saturating_sub(overhead) as usize;

    let mut lines: Vec<Line> = Vec::new();

    // Filter input line
    if state.filter.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(" /", Style::default().fg(palette.dimmed)),
            Span::styled("_", Style::default().fg(palette.dimmed)),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(" /", Style::default().fg(palette.dimmed)),
            Span::styled(state.filter.clone(), Style::default().fg(palette.text)),
            Span::styled("_", Style::default().fg(palette.text)),
        ]));
    }

    lines.push(Line::from(""));

    if is_pr_mode {
        // PR mode: show PR list
        match &state.pr_list {
            Some(PrListState::Loading) => {
                lines.push(Line::from(vec![Span::styled(
                    " Loading PRs...",
                    Style::default().fg(palette.dimmed),
                )]));
                for _ in 1..max_visible {
                    lines.push(Line::from(""));
                }
            }
            Some(PrListState::Loaded { prs, .. }) => {
                let filtered = state.filtered_prs();
                if filtered.is_empty() {
                    lines.push(Line::from(vec![Span::styled(
                        if state.filter.is_empty() {
                            " No open PRs."
                        } else {
                            " No matching PRs."
                        },
                        Style::default().fg(palette.dimmed),
                    )]));
                    for _ in 1..max_visible {
                        lines.push(Line::from(""));
                    }
                } else {
                    let total = filtered.len();
                    let start = if total <= max_visible || state.cursor < max_visible / 2 {
                        0
                    } else if state.cursor + max_visible / 2 >= total {
                        total.saturating_sub(max_visible)
                    } else {
                        state.cursor - max_visible / 2
                    };
                    let end = (start + max_visible).min(total);

                    for (fi, &idx) in filtered.iter().enumerate().take(end).skip(start) {
                        let pr = &prs[idx];
                        let is_selected = fi == state.cursor;
                        let cursor_str = if is_selected { "> " } else { "  " };

                        let title_style = if is_selected {
                            Style::default().fg(palette.accent)
                        } else {
                            Style::default().fg(palette.text)
                        };

                        let mut spans = vec![
                            Span::styled(cursor_str, Style::default().fg(palette.text)),
                            Span::styled(
                                format!("#{} ", pr.number),
                                Style::default().fg(palette.dimmed),
                            ),
                            Span::styled(pr.title.clone(), title_style),
                        ];
                        if pr.is_draft {
                            spans.push(dim(" [draft]"));
                        }

                        lines.push(Line::from(spans));
                    }

                    for _ in (end - start)..max_visible {
                        lines.push(Line::from(""));
                    }
                }
            }
            Some(PrListState::Error { message }) => {
                lines.push(Line::from(vec![Span::styled(
                    format!(" {}", message),
                    Style::default().fg(palette.danger),
                )]));
                for _ in 1..max_visible {
                    lines.push(Line::from(""));
                }
            }
            None => {
                for _ in 0..max_visible {
                    lines.push(Line::from(""));
                }
            }
        }
    } else {
        // Branch mode
        let filtered = state.filtered();

        // Action row: "Create" or "Checkout PR #N"
        if !state.filter.trim().is_empty() {
            let cursor_str = if state.cursor == 0 { "> " } else { "  " };
            let action_style = if state.cursor == 0 {
                Style::default()
                    .fg(palette.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.text)
            };

            let label = if let Some(pr_num) = state.detected_pr_number() {
                format!("+ Checkout PR #{}", pr_num)
            } else {
                format!("+ Create \"{}\"", state.filter.trim())
            };

            lines.push(Line::from(vec![
                Span::styled(cursor_str, Style::default().fg(palette.text)),
                Span::styled(label, action_style),
            ]));
        }

        // Branch rows
        if filtered.is_empty() && state.filter.trim().is_empty() {
            lines.push(Line::from(vec![Span::styled(
                " Type to search or create...",
                Style::default().fg(palette.dimmed),
            )]));
            for _ in 1..max_visible {
                lines.push(Line::from(""));
            }
        } else if filtered.is_empty() {
            for _ in 0..max_visible {
                lines.push(Line::from(""));
            }
        } else {
            let has_create_row = !state.filter.trim().is_empty();
            let branch_cursor = if has_create_row {
                state.cursor.checked_sub(1)
            } else {
                Some(state.cursor)
            };

            let total = filtered.len();
            let effective_cursor = branch_cursor.unwrap_or(0);
            let start = if total <= max_visible || effective_cursor < max_visible / 2 {
                0
            } else if effective_cursor + max_visible / 2 >= total {
                total.saturating_sub(max_visible)
            } else {
                effective_cursor - max_visible / 2
            };
            let end = (start + max_visible).min(total);

            for (fi, &idx) in filtered.iter().enumerate().take(end).skip(start) {
                let branch = &state.branches[idx];
                let is_selected = branch_cursor == Some(fi);
                let cursor_str = if is_selected { "> " } else { "  " };
                let is_occupied = state.occupied_branches.contains(branch);

                let branch_style = if is_occupied {
                    Style::default().fg(palette.dimmed)
                } else if is_selected {
                    Style::default().fg(palette.accent)
                } else {
                    Style::default().fg(palette.text)
                };

                let mut spans = vec![
                    Span::styled(cursor_str, Style::default().fg(palette.text)),
                    Span::styled(branch.clone(), branch_style),
                ];
                if is_occupied {
                    spans.push(dim(" (in use)"));
                }

                lines.push(Line::from(spans));
            }

            for _ in (end - start)..max_visible {
                lines.push(Line::from(""));
            }
        }
    }

    lines.push(Line::from(""));

    // Footer (mode-dependent)
    if is_pr_mode {
        lines.push(Line::from(vec![
            Span::raw(" "),
            bold("Enter"),
            dim(" checkout  "),
            bold("^p"),
            dim(" branches  "),
            bold("Esc"),
            dim(" cancel"),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::raw(" "),
            bold("Enter"),
            dim(" select  "),
            bold("^b"),
            dim(" base  "),
            bold("^p"),
            dim(" PRs  "),
            bold("Esc"),
            dim(" cancel"),
        ]));
    }
    lines.push(Line::from(""));

    let popup_area = Rect {
        x: area.width.saturating_sub(width) / 2,
        y: area.height.saturating_sub(height) / 2,
        width: width.min(area.width),
        height: height.min(area.height),
    };

    // Title and bottom border
    let title_text = if is_pr_mode {
        "Checkout PR"
    } else {
        "Add Worktree"
    };

    let mut block = Block::bordered()
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(palette.help_border))
        .title(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                title_text,
                Style::default()
                    .fg(palette.header)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default()),
        ]));

    // Show base branch on bottom border only in branch mode
    if !is_pr_mode {
        let base_title = if state.editing_base {
            Line::from(vec![
                Span::styled(" Base: ", Style::default().fg(palette.dimmed)),
                Span::styled(
                    state.base_filter.clone(),
                    Style::default().fg(palette.accent),
                ),
                Span::styled("_ ", Style::default().fg(palette.accent)),
            ])
        } else {
            Line::from(vec![
                Span::styled(" Base: ", Style::default().fg(palette.dimmed)),
                Span::styled(
                    format!("{} ", state.base_branch),
                    Style::default().fg(palette.text),
                ),
            ])
        };
        block = block.title_bottom(base_title);
    }

    let paragraph = Paragraph::new(Text::from(lines)).block(block);

    f.render_widget(Clear, popup_area);
    f.render_widget(paragraph, popup_area);
}
