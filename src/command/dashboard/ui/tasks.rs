//! Task graph tab rendering.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap},
};

use crate::command::dashboard::app::{App, TaskFormField, TaskModal};
use crate::tasks::types::{STATUS_DONE, STATUS_FAILED, STATUS_IN_PROGRESS, STATUS_TODO};

fn status_icon(status: &str) -> (&'static str, Color) {
    match status {
        STATUS_TODO => ("○", Color::DarkGray),
        STATUS_IN_PROGRESS => ("◐", Color::Yellow),
        STATUS_DONE => ("●", Color::Green),
        STATUS_FAILED => ("✗", Color::Red),
        _ => ("?", Color::DarkGray),
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}

pub fn render_task_table(f: &mut Frame, app: &mut App, area: Rect) {
    let rows: Vec<Row> = app
        .tasks
        .filtered_indices
        .iter()
        .map(|&i| {
            let t = &app.tasks.all_tasks[i];
            let (icon, color) = status_icon(&t.status);
            let deps = if t.depends_on.is_empty() {
                "—".to_string()
            } else {
                t.depends_on.join(", ")
            };
            let worktree = t.worktree.as_deref().unwrap_or("—").to_string();
            Row::new(vec![
                Cell::from(Span::styled(icon, Style::default().fg(color))),
                Cell::from(t.id.clone()),
                Cell::from(t.title.clone()),
                Cell::from(t.status.clone()),
                Cell::from(worktree),
                Cell::from(deps),
            ])
        })
        .collect();

    let header = Row::new(vec!["", "ID", "Title", "Status", "Worktree", "Depends on"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let p = &app.palette;
    let table = Table::new(
        rows,
        [
            Constraint::Length(3),
            Constraint::Length(20),
            Constraint::Min(20),
            Constraint::Length(12),
            Constraint::Length(16),
            Constraint::Min(16),
        ],
    )
    .header(header)
    .row_highlight_style(Style::default().bg(p.highlight_row_bg))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(p.border))
            .title(Span::styled(
                format!(
                    " Tasks ({}/{}) ",
                    app.tasks.filtered_indices.len(),
                    app.tasks.all_tasks.len()
                ),
                Style::default().fg(p.header).add_modifier(Modifier::BOLD),
            )),
    );

    f.render_stateful_widget(table, area, &mut app.tasks.table_state);
}

pub fn render_task_detail(f: &mut Frame, app: &App, area: Rect) {
    let p = &app.palette;
    let Some(task) = app.task_selected_task() else {
        f.render_widget(
            Paragraph::new("no task selected")
                .style(Style::default().fg(p.dimmed))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(p.border))
                        .title(Span::styled(" Detail ", Style::default().fg(p.header))),
                ),
            area,
        );
        return;
    };

    let blocks: Vec<&str> = app
        .tasks
        .all_tasks
        .iter()
        .filter(|t| t.depends_on.iter().any(|d| d == &task.id))
        .map(|t| t.id.as_str())
        .collect();

    let dep_lines: Vec<String> = if task.depends_on.is_empty() {
        vec!["—".to_string()]
    } else {
        task.depends_on
            .iter()
            .map(|dep| {
                let st = app
                    .tasks
                    .all_tasks
                    .iter()
                    .find(|t| &t.id == dep)
                    .map(|t| t.status.as_str())
                    .unwrap_or("?");
                let (icon, _) = status_icon(st);
                format!("{icon} {dep} [{st}]")
            })
            .collect()
    };

    let blocks_str = if blocks.is_empty() {
        "(none)".to_string()
    } else {
        blocks.join(", ")
    };

    let desc = if task.description.is_empty() {
        "(no description)".to_string()
    } else {
        task.description.clone()
    };

    let worktree_str = task
        .worktree
        .as_deref()
        .map(|w| format!("Worktree: {w}"))
        .unwrap_or_else(|| "Worktree: (not linked)".to_string());

    let text = format!(
        "{}\n\n{}\nDepends on: {}  |  Blocks: {}",
        desc,
        worktree_str,
        dep_lines.join(", "),
        blocks_str
    );

    f.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(p.text))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(p.border))
                    .title(Span::styled(
                        format!(" {} — {} ", task.id, task.status),
                        Style::default().fg(p.header),
                    )),
            ),
        area,
    );
}

pub fn render_task_footer(f: &mut Frame, app: &App, area: Rect) {
    let p = &app.palette;
    let dimmed = Style::default().fg(p.dimmed);
    let bold = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let pipe = Span::styled(" │ ", Style::default().fg(p.border));

    let cmd = |k: &'static str, l: &'static str| -> Vec<Span<'static>> {
        vec![Span::styled(k, dimmed), Span::styled(format!(" {l}"), bold)]
    };

    let filter_label = if app.tasks.filter_text.is_empty() {
        "filter".to_string()
    } else {
        format!("/{}", app.tasks.filter_text)
    };

    let status_label = app.tasks.status_filter.label();

    let mut spans: Vec<Span> = vec![Span::raw("  ")];
    spans.extend(cmd("a", "Add"));
    spans.push(pipe.clone());
    spans.extend(cmd("e", "Edit"));
    spans.push(pipe.clone());
    spans.extend(cmd("D", "Delete"));
    spans.push(pipe.clone());
    spans.extend(cmd("s", "Cycle status"));
    spans.push(pipe.clone());
    spans.extend(cmd("I", "Start impl"));
    spans.push(pipe.clone());
    spans.push(Span::styled("f", dimmed));
    spans.push(Span::styled(format!(" filter({status_label})"), bold));
    spans.push(pipe.clone());
    spans.push(Span::styled("/", dimmed));
    spans.push(Span::styled(format!(" {filter_label}"), bold));
    spans.push(pipe.clone());
    spans.extend(cmd("Tab", "Switch tab"));
    spans.push(pipe.clone());
    spans.extend(cmd("q", "Quit"));

    let right = Line::from(vec![
        Span::styled("?", dimmed),
        Span::styled(" Help ", bold),
    ]);
    let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Length(7)]).split(area);
    f.render_widget(Paragraph::new(Line::from(spans)), cols[0]);
    f.render_widget(Paragraph::new(right), cols[1]);
}

pub fn render_task_footer_filter(f: &mut Frame, app: &App, area: Rect) {
    let p = &app.palette;
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  /",
                Style::default().fg(p.keycap).add_modifier(Modifier::BOLD),
            ),
            Span::raw(app.tasks.filter_text.as_str()),
            Span::styled("_", Style::default().fg(p.keycap)),
            Span::raw("  "),
            Span::styled("Enter", Style::default().fg(p.dimmed)),
            Span::raw(" accept  "),
            Span::styled("Esc", Style::default().fg(p.dimmed)),
            Span::raw(" clear"),
        ])),
        area,
    );
}

pub fn render_task_modals(f: &mut Frame, app: &App, area: Rect) {
    match &app.tasks.modal {
        Some(TaskModal::Form(_)) => render_form_modal(f, app, area),
        Some(TaskModal::DeleteConfirm(_)) => render_delete_modal(f, app, area),
        Some(TaskModal::Help) => render_help_modal(f, area),
        None => {}
    }
}

fn render_form_modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(TaskModal::Form(form)) = &app.tasks.modal else {
        return;
    };
    let modal_area = centered_rect(70, 90, area);
    f.render_widget(Clear, modal_area);

    let title = if form.is_edit {
        " Edit Task "
    } else {
        " Add Task "
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(5), // Title (word-wrapped, 3 visible lines)
            Constraint::Min(3),    // Description (expands)
            Constraint::Length(3), // ID
            Constraint::Length(3), // DependsOn
            Constraint::Length(3), // Worktree
            Constraint::Length(3), // Status
            Constraint::Length(1), // error/hint
        ])
        .split(modal_area);

    f.render_widget(
        Block::default().borders(Borders::ALL).title(title),
        modal_area,
    );

    // Render TextArea fields
    let ta_field_data: [(TaskFormField, &str, &tui_textarea::TextArea); 5] = [
        (TaskFormField::Title, "Title", &form.title),
        (TaskFormField::Description, "Description", &form.description),
        (
            TaskFormField::Id,
            if form.id_auto { "ID (auto)" } else { "ID" },
            &form.id,
        ),
        (
            TaskFormField::DependsOn,
            "Depends on (space-separated IDs)",
            &form.depends_on,
        ),
        (
            TaskFormField::Worktree,
            "Linked worktree (handle)",
            &form.worktree,
        ),
    ];

    for (i, (field, label, ta)) in ta_field_data.iter().enumerate() {
        let is_focused = form.focused_field() == *field;
        let border_style = if is_focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let mut ta_clone = (*ta).clone();
        ta_clone.set_block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(*label),
        );
        if is_focused {
            ta_clone.set_cursor_line_style(Style::default().fg(Color::Yellow));
        } else {
            ta_clone.set_cursor_line_style(Style::default());
            ta_clone.set_cursor_style(Style::default()); // hide cursor when not focused
        }
        f.render_widget(&ta_clone, chunks[i]);
    }

    // Status field (plain Paragraph, cycled with Space)
    {
        let is_focused = form.focused_field() == TaskFormField::Status;
        let style = if is_focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        let border_style = if is_focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        f.render_widget(
            Paragraph::new(form.status.as_str()).style(style).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title("Status (Space to cycle)"),
            ),
            chunks[5],
        );
    }

    // Dep suggestions popup (shown when DependsOn is focused and suggestions exist)
    if form.focused_field() == TaskFormField::DependsOn && !form.dep_suggestions.is_empty() {
        render_dep_suggestions(f, form, chunks[3]);
    }

    if let Some(ref err) = form.error {
        f.render_widget(
            Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red)),
            chunks[6],
        );
    } else {
        f.render_widget(
            Paragraph::new("Tab: next field / cycle suggestions  Enter: confirm  Shift+Enter: newline in desc  Esc: cancel")
                .style(Style::default().fg(Color::DarkGray)),
            chunks[6],
        );
    }
}

fn render_dep_suggestions(
    f: &mut Frame,
    form: &crate::command::dashboard::app::TaskForm,
    field_area: Rect,
) {
    let max_shown = 5usize;
    let count = form.dep_suggestions.len().min(max_shown);
    let popup_height = count as u16 + 2; // border top+bottom
    let popup_width = field_area.width.min(40);

    // Place popup just below the field
    let popup_y = field_area.y + field_area.height;
    let popup_x = field_area.x + 2;

    // Clamp to screen
    let popup_area = Rect {
        x: popup_x,
        y: popup_y,
        width: popup_width,
        height: popup_height,
    };

    f.render_widget(Clear, popup_area);
    f.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" suggestions "),
        popup_area,
    );

    let inner = Rect {
        x: popup_area.x + 1,
        y: popup_area.y + 1,
        width: popup_area.width.saturating_sub(2),
        height: popup_area.height.saturating_sub(2),
    };

    for (i, id) in form.dep_suggestions.iter().take(max_shown).enumerate() {
        let is_selected = i == form.dep_suggestion_cursor;
        let style = if is_selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let row_area = Rect {
            x: inner.x,
            y: inner.y + i as u16,
            width: inner.width,
            height: 1,
        };
        f.render_widget(Paragraph::new(id.as_str()).style(style), row_area);
    }
}

fn render_delete_modal(f: &mut Frame, app: &App, area: Rect) {
    let Some(TaskModal::DeleteConfirm(plan)) = &app.tasks.modal else {
        return;
    };
    let modal_area = centered_rect(55, 30, area);
    f.render_widget(Clear, modal_area);

    let mut lines: Vec<Line> = vec![
        Line::from(Span::styled(
            format!("Delete task '{}'?", plan.id),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if let Some(ref handle) = plan.worktree {
        let (toggle_label, toggle_style) = if plan.delete_worktree {
            (
                format!("[w] worktree + branch: will be deleted  ('{}')", handle),
                Style::default().fg(Color::Red),
            )
        } else {
            (
                format!("[w] worktree + branch: will be kept  ('{}')", handle),
                Style::default().fg(Color::Green),
            )
        };
        lines.push(Line::from(Span::styled(toggle_label, toggle_style)));
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "[y] confirm  [n/Esc] cancel",
        Style::default().fg(Color::DarkGray),
    )));

    f.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Delete Task ")
                .border_style(Style::default().fg(Color::Red)),
        ),
        modal_area,
    );
}

fn render_help_modal(f: &mut Frame, area: Rect) {
    let modal_area = centered_rect(60, 70, area);
    f.render_widget(Clear, modal_area);
    let help = "\
j / ↓       navigate down
k / ↑       navigate up
/           enter filter mode
Esc         clear filter / close
f           cycle status filter
s           cycle task status
I           start implementation (new worktree)
a           add new task
e           edit selected task
D           delete selected task
r           reload from file
Tab         switch tab
q           quit
?           toggle this help";
    f.render_widget(
        Paragraph::new(help).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Help  (any key to close) "),
        ),
        modal_area,
    );
}
