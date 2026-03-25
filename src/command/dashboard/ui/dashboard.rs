//! Dashboard view rendering (table, preview, footer).

use ansi_to_tui::IntoText;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Cell, Paragraph, Row, Table},
};
use std::collections::{BTreeMap, HashSet};

use super::super::app::{App, DashboardTab};
use super::super::spinner::SPINNER_FRAMES;
use super::format;
use super::format::{format_git_status, format_pr_status};
use super::worktree::{render_worktree_preview, render_worktree_table};

/// Render the tab header line showing Agents | Worktrees with active tab highlighted.
fn render_tab_header(f: &mut Frame, app: &App, area: Rect) {
    let active_style = Style::default()
        .fg(app.palette.header)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().fg(app.palette.dimmed);
    let pipe_style = Style::default().fg(app.palette.border);
    let rule_style = Style::default().fg(app.palette.border);

    let (agents_style, worktrees_style) = match app.active_tab {
        DashboardTab::Agents => (active_style, inactive_style),
        DashboardTab::Worktrees => (inactive_style, active_style),
    };

    let tabs = Line::from(vec![
        Span::raw("  "),
        Span::styled("Agents", agents_style),
        Span::styled(" \u{2502} ", pipe_style),
        Span::styled("Worktrees", worktrees_style),
    ]);
    let rule = Line::from(Span::styled(
        "\u{2500}".repeat(area.width as usize),
        rule_style,
    ));

    f.render_widget(Paragraph::new(vec![tabs, rule]), area);
}

/// Render the dashboard view (table + preview + footer).
pub fn render_dashboard(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Check if backend supports preview
    let supports_preview = app.mux.supports_preview();

    // Outer layout: fixed-height tab header and footer, flexible content area.
    // Fill(1) guarantees the content takes exactly the remaining space.
    let outer = Layout::vertical([
        Constraint::Length(2), // Tab header + spacer
        Constraint::Fill(1),   // Content (table + optional preview)
        Constraint::Length(1), // Footer
    ])
    .split(area);

    let tab_area = outer[0];
    let content_area = outer[1];
    let footer_area = outer[2];

    // Split content area into table + preview (or just table if no preview)
    let (table_area, preview_area) = if !supports_preview {
        (content_area, None)
    } else {
        let table_size = 100u16.saturating_sub(app.preview_size as u16);
        // Use Fill() proportional constraints to split space safely without overflow
        let content_chunks = Layout::vertical([
            Constraint::Fill(table_size),              // Table
            Constraint::Fill(app.preview_size as u16), // Preview
        ])
        .split(content_area);
        (content_chunks[0], Some(content_chunks[1]))
    };

    // Tab header
    render_tab_header(f, app, tab_area);

    // Table (agents or worktrees based on active tab)
    match app.active_tab {
        DashboardTab::Agents => render_table(f, app, table_area),
        DashboardTab::Worktrees => render_worktree_table(f, app, table_area),
    }

    // Preview (only for backends that support it)
    if let Some(preview) = preview_area {
        match app.active_tab {
            DashboardTab::Agents => render_preview(f, app, preview),
            DashboardTab::Worktrees => render_worktree_preview(f, app, preview),
        }
    }

    // Footer - show status message if active, otherwise mode-specific help
    if let Some((msg, _)) = &app.status_message {
        let p = &app.palette;
        f.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                format!("  {msg}"),
                Style::default().fg(p.text),
            )])),
            footer_area,
        );
    } else {
        match app.active_tab {
            DashboardTab::Agents => {
                if app.filter_active {
                    f.render_widget(render_footer_filter(app), footer_area);
                } else if app.input_mode {
                    f.render_widget(render_footer_input(app), footer_area);
                } else {
                    render_footer_normal(f, app, footer_area);
                }
            }
            DashboardTab::Worktrees => {
                if app.worktree_filter_active {
                    f.render_widget(render_worktree_footer_filter(app), footer_area);
                } else {
                    render_worktree_footer_normal(f, app, footer_area);
                }
            }
        }
    }
}

fn render_table(f: &mut Frame, app: &mut App, area: Rect) {
    // Check if we should show the PR column (only when at least one agent has a PR)
    let show_pr_column = app.has_any_pr();
    let show_check_counts = app.config.dashboard.show_check_counts();

    // Check if git data is being refreshed
    let is_git_fetching = app
        .is_git_fetching
        .load(std::sync::atomic::Ordering::Relaxed);

    // Build header with spinner in Git column when fetching
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

    // Build PR header with spinner when fetching
    let pr_header = if app.is_pr_fetching() {
        let spinner = SPINNER_FRAMES[app.spinner_frame as usize % SPINNER_FRAMES.len()];
        Line::from(vec![
            Span::styled("PR ", Style::default().fg(app.palette.header).bold()),
            Span::styled(spinner.to_string(), Style::default().fg(app.palette.dimmed)),
        ])
    } else {
        Line::from(Span::styled(
            "PR",
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
        header_cells.push(Cell::from(pr_header));
    }

    header_cells.extend(vec![
        Cell::from("Status").style(header_style),
        Cell::from("Time").style(header_style),
        Cell::from("Title").style(header_style),
    ]);

    let header = Row::new(header_cells).height(1);

    // Group agents by (session, window_name) to detect multi-pane windows
    let mut window_groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (idx, agent) in app.agents.iter().enumerate() {
        let key = (agent.session.clone(), agent.window_name.clone());
        window_groups.entry(key).or_default().push(idx);
    }

    // Build a set of windows with multiple panes
    let multi_pane_windows: HashSet<(String, String)> = window_groups
        .iter()
        .filter(|(_, indices)| indices.len() > 1)
        .map(|(key, _)| key.clone())
        .collect();

    // Track position within each window group for pane numbering
    let mut window_positions: BTreeMap<(String, String), usize> = BTreeMap::new();

    // Pre-compute row data to calculate max widths
    let row_data: Vec<_> = app
        .agents
        .iter()
        .enumerate()
        .map(|(idx, agent)| {
            let key = (agent.session.clone(), agent.window_name.clone());
            let is_multi_pane = multi_pane_windows.contains(&key);

            let pane_suffix = if is_multi_pane {
                let pos = window_positions.entry(key.clone()).or_insert(0);
                *pos += 1;
                format!(" [{}]", pos)
            } else {
                String::new()
            };

            let jump_key = if idx < 9 {
                format!("{}", idx + 1)
            } else {
                String::new()
            };

            let project = App::extract_project_name(agent);
            let (worktree_name, is_main) = app.extract_worktree_name(agent);
            // Check if this agent corresponds to the current working directory.
            // Try canonicalized comparison first (handles symlinks), fall back to direct comparison.
            let is_current = app.current_worktree.as_ref().is_some_and(|cwd| {
                // Try canonical comparison first (resolves symlinks like /var -> /private/var on macOS)
                if let (Ok(cwd_canonical), Ok(agent_canonical)) =
                    (cwd.canonicalize(), agent.path.canonicalize())
                {
                    cwd_canonical == agent_canonical
                } else {
                    // Fall back to direct comparison
                    agent.path == *cwd
                }
            });
            let worktree_display = format!("{}{}", worktree_name, pane_suffix);
            let title = agent
                .pane_title
                .as_ref()
                .map(|t| t.strip_prefix("... ").unwrap_or(t).to_string())
                .unwrap_or_default();
            let status_spans = app.get_status_display(agent);
            let duration = app
                .get_elapsed(agent)
                .map(|d| app.format_duration(d))
                .unwrap_or_else(|| "-".to_string());

            // Get git status for this worktree (may be None if not yet fetched)
            let git_status = app.git_statuses.get(&agent.path);
            let git_spans = format_git_status(git_status, app.spinner_frame, &app.palette);

            // Get PR status for this agent (only if column is shown)
            let pr_spans = if show_pr_column {
                let pr = app.get_pr_for_agent(agent);
                Some(format_pr_status(
                    pr,
                    show_check_counts,
                    app.spinner_frame,
                    &app.palette,
                ))
            } else {
                None
            };

            (
                jump_key,
                project,
                worktree_display,
                is_main,
                is_current,
                git_spans,
                pr_spans,
                status_spans,
                duration,
                title,
            )
        })
        .collect();

    // Calculate max project name width (with padding, capped)
    let max_project_width = row_data
        .iter()
        .map(|(_, project, _, _, _, _, _, _, _, _)| project.len())
        .max()
        .unwrap_or(5)
        .clamp(5, 20) // min 5, max 20
        + 2; // padding

    // Calculate max worktree name width (with padding)
    // Use at least 8 to fit the "Worktree" header
    let max_worktree_width = row_data
        .iter()
        .map(|(_, _, worktree_display, _, _, _, _, _, _, _)| worktree_display.len())
        .max()
        .unwrap_or(8)
        .max(8) // min 8 (header width)
        + 1; // padding

    // Calculate max git status width (sum of all span character counts)
    // Use chars().count() instead of len() because Nerd Font icons are multi-byte
    let max_git_width = row_data
        .iter()
        .map(|(_, _, _, _, _, git_spans, _, _, _, _)| {
            git_spans
                .iter()
                .map(|(text, _)| text.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(4)
        .clamp(4, 30) // min 4, max 30 (increased for base branch)
        + 1; // padding

    // Calculate max PR status width (only if showing PR column)
    let max_pr_width = if show_pr_column {
        row_data
            .iter()
            .filter_map(|(_, _, _, _, _, _, pr_spans, _, _, _)| pr_spans.as_ref())
            .map(|spans| {
                spans
                    .iter()
                    .map(|(text, _)| text.chars().count())
                    .sum::<usize>()
            })
            .max()
            .unwrap_or(4)
            .clamp(4, 20) // Accommodate check icons + counts + inline timer
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
                status_spans,
                duration,
                title,
            )| {
                let worktree_style = if is_current {
                    Style::default().fg(app.palette.current_worktree_fg)
                } else if is_main {
                    Style::default().fg(app.palette.dimmed)
                } else {
                    Style::default()
                };
                // Convert git spans to a Line
                let git_line = Line::from(
                    git_spans
                        .into_iter()
                        .map(|(text, style)| Span::styled(text, style))
                        .collect::<Vec<_>>(),
                );

                let mut cells = vec![
                    Cell::from(jump_key).style(Style::default().fg(app.palette.keycap)),
                    Cell::from(project),
                    Cell::from(worktree_display).style(worktree_style),
                    Cell::from(git_line),
                ];

                // Add PR cell if column is shown
                if let Some(pr_spans) = pr_spans {
                    let pr_line = Line::from(
                        pr_spans
                            .into_iter()
                            .map(|(text, style)| Span::styled(text, style))
                            .collect::<Vec<_>>(),
                    );
                    cells.push(Cell::from(pr_line));
                }

                let status_line = Line::from(
                    status_spans
                        .into_iter()
                        .map(|(text, style)| Span::styled(text, style))
                        .collect::<Vec<_>>(),
                );
                cells.extend(vec![
                    Cell::from(status_line),
                    Cell::from(duration),
                    Cell::from(title),
                ]);

                let row = Row::new(cells);
                // Subtle background for the active worktree row
                if is_current {
                    row.style(Style::default().bg(app.palette.current_row_bg))
                } else {
                    row
                }
            },
        )
        .collect();

    // Build column constraints conditionally based on whether PR column is shown
    let mut constraints = vec![
        Constraint::Length(2),                         // #: jump key
        Constraint::Length(max_project_width as u16),  // Project: auto-sized
        Constraint::Length(max_worktree_width as u16), // Worktree: auto-sized
        Constraint::Length(max_git_width as u16),      // Git: auto-sized
    ];

    if show_pr_column {
        constraints.push(Constraint::Length(max_pr_width as u16)); // PR: auto-sized
    }

    constraints.extend(vec![
        Constraint::Length(8),  // Status: fixed (icons)
        Constraint::Length(10), // Time: HH:MM:SS + padding
        Constraint::Fill(1),    // Title: takes remaining space
    ]);

    let table = Table::new(rows, constraints)
        .header(header)
        .block(Block::default())
        .row_highlight_style(Style::default().bg(app.palette.highlight_row_bg))
        .highlight_symbol("> ");

    f.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_preview(f: &mut Frame, app: &mut App, area: Rect) {
    // Get info about the selected agent for the title
    let selected_agent = app
        .table_state
        .selected()
        .and_then(|idx| app.agents.get(idx));

    let (title, title_style, border_style) = if app.input_mode {
        let worktree_name = selected_agent
            .map(|a| app.extract_worktree_name(a).0)
            .unwrap_or_default();
        (
            format!(" INPUT: {} ", worktree_name),
            Style::default()
                .fg(app.palette.success)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(app.palette.success),
        )
    } else if let Some(agent) = selected_agent {
        let worktree_name = app.extract_worktree_name(agent).0;
        (
            format!(" Preview: {} ", worktree_name),
            Style::default().fg(app.palette.header),
            Style::default().fg(app.palette.border),
        )
    } else {
        (
            " Preview ".to_string(),
            Style::default().fg(app.palette.header),
            Style::default().fg(app.palette.border),
        )
    };

    let mut block = Block::bordered()
        .title(title)
        .title_style(title_style)
        .border_style(border_style);

    // Add right-aligned PR check detail to the preview border
    if let Some(agent) = selected_agent
        && let Some(pr) = app.get_pr_for_agent(agent)
    {
        let detail_spans = format::format_pr_details(pr, app.spinner_frame, &app.palette);
        if !detail_spans.is_empty() {
            let mut title_spans = vec![Span::raw(" ")];
            title_spans.extend(detail_spans);
            title_spans.push(Span::raw(" "));
            block = block.title_top(Line::from(title_spans).right_aligned());
        }
    }

    // Calculate the inner area to determine scroll offset
    let inner_area = block.inner(area);

    // Update preview height for scroll calculations
    app.preview_height = inner_area.height;

    // Get preview content or show placeholder
    let (text, line_count) = match (&app.preview, selected_agent) {
        (Some(preview), Some(_)) => {
            let trimmed = preview.trim_end();
            if trimmed.is_empty() {
                (Text::raw("(empty output)"), 1u16)
            } else {
                // Parse ANSI escape sequences to get colored text
                match trimmed.into_text() {
                    Ok(text) => {
                        let count = text.lines.len() as u16;
                        (text, count)
                    }
                    Err(_) => {
                        // Fallback: strip ANSI escapes to prevent raw control
                        // sequences from corrupting the terminal display
                        let safe = super::super::ansi::strip_ansi_escapes(trimmed);
                        let count = safe.lines().count() as u16;
                        (Text::raw(safe), count)
                    }
                }
            }
        }
        (None, Some(_)) => (Text::raw("(pane not available)"), 1),
        (_, None) => (Text::raw("(no agent selected)"), 1),
    };

    // Update line count for scroll calculations
    app.preview_line_count = line_count;

    // Calculate scroll offset: use manual scroll if set, otherwise auto-scroll to bottom
    let max_scroll = line_count.saturating_sub(inner_area.height);
    let scroll_offset = app.preview_scroll.unwrap_or(max_scroll);

    let paragraph = Paragraph::new(text).block(block).scroll((scroll_offset, 0));

    f.render_widget(paragraph, area);
}

// ── Footer rendering ────────────────────────────────────────────

/// Filter mode footer
fn render_footer_filter<'a>(app: &'a App) -> Paragraph<'a> {
    Paragraph::new(Line::from(vec![
        Span::styled(
            "  /",
            Style::default()
                .fg(app.palette.keycap)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(app.filter_text.as_str()),
        Span::styled("_", Style::default().fg(app.palette.keycap)),
        Span::raw("  "),
        Span::styled("Enter", Style::default().fg(app.palette.dimmed)),
        Span::raw(" accept  "),
        Span::styled("Esc", Style::default().fg(app.palette.dimmed)),
        Span::raw(" clear"),
    ]))
}

/// Input mode footer
fn render_footer_input<'a>(app: &'a App) -> Paragraph<'a> {
    Paragraph::new(Line::from(vec![
        Span::styled(
            "  INPUT MODE",
            Style::default()
                .fg(app.palette.success)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" \u{2014} type to send keys to agent  "),
        Span::styled("Esc", Style::default().fg(app.palette.keycap)),
        Span::raw(" exit"),
    ]))
}

/// Normal mode footer with right-pinned help
fn render_footer_normal(f: &mut Frame, app: &App, area: Rect) {
    let p = &app.palette;

    let dimmed = Style::default().fg(p.dimmed);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let pipe_style = Style::default().fg(p.border);
    let active_style = Style::default().fg(p.info);

    let cmd = |k: String, l: String| -> Vec<Span<'static>> {
        vec![
            Span::styled(k, dimmed),
            Span::styled(format!(" {}", l), bold_text),
        ]
    };
    let toggle = |k: String, l: String, v: String, active: bool| -> Vec<Span<'static>> {
        vec![
            Span::styled(k, dimmed),
            Span::styled(format!(" {} ", l), bold_text),
            Span::styled(
                format!("({})", v),
                if active { active_style } else { dimmed },
            ),
        ]
    };
    let pipe = || -> Span<'static> { Span::styled(" \u{2502} ", pipe_style) };

    let sort = app.sort_mode.label();
    let scope = app.scope_mode.label();
    let stale = if app.hide_stale { "hidden" } else { "shown" };
    let scope_active = scope != "all";
    let stale_active = stale == "hidden";

    let mut s: Vec<Span<'static>> = vec![Span::raw("  ")];
    s.extend(cmd("i".into(), "Input".into()));
    s.push(pipe());
    s.extend(cmd("d".into(), "Diff".into()));
    s.push(pipe());
    s.extend(cmd("o".into(), "PR".into()));
    s.push(pipe());
    s.extend(cmd("1-9".into(), "Jump".into()));
    s.push(pipe());
    s.extend(toggle("s".into(), "Sort".into(), sort.to_string(), true));
    s.push(pipe());
    s.extend(toggle(
        "F".into(),
        "Scope".into(),
        scope.to_string(),
        scope_active,
    ));
    s.push(pipe());
    s.extend(toggle(
        "f".into(),
        "Stale".into(),
        stale.to_string(),
        stale_active,
    ));
    if !app.filter_text.is_empty() {
        s.push(pipe());
        s.extend(cmd("/".into(), app.filter_text.clone()));
    }
    s.push(pipe());
    s.extend(cmd("Tab".into(), "Worktrees".into()));
    s.push(pipe());
    s.extend(cmd("q".into(), "Quit".into()));

    // Split footer: left commands, right-pinned help
    let right = Line::from(vec![
        Span::styled("?", dimmed),
        Span::styled(" Help ", bold_text),
    ]);
    let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Length(7)]).split(area);

    f.render_widget(Paragraph::new(Line::from(s)), cols[0]);
    f.render_widget(Paragraph::new(right), cols[1]);
}

/// Worktree filter mode footer
fn render_worktree_footer_filter<'a>(app: &'a App) -> Paragraph<'a> {
    Paragraph::new(Line::from(vec![
        Span::styled(
            "  /",
            Style::default()
                .fg(app.palette.keycap)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(app.worktree_filter_text.as_str()),
        Span::styled("_", Style::default().fg(app.palette.keycap)),
        Span::raw("  "),
        Span::styled("Enter", Style::default().fg(app.palette.dimmed)),
        Span::raw(" accept  "),
        Span::styled("Esc", Style::default().fg(app.palette.dimmed)),
        Span::raw(" clear"),
    ]))
}

/// Worktree normal mode footer
fn render_worktree_footer_normal(f: &mut Frame, app: &App, area: Rect) {
    let p = &app.palette;

    let dimmed = Style::default().fg(p.dimmed);
    let bold_text = Style::default().fg(p.text).add_modifier(Modifier::BOLD);
    let active_style = Style::default().fg(p.accent);
    let pipe_style = Style::default().fg(p.border);

    let cmd = |k: String, l: String| -> Vec<Span<'static>> {
        vec![
            Span::styled(k, dimmed),
            Span::styled(format!(" {}", l), bold_text),
        ]
    };
    let toggle = |k: String, l: String, v: String, active: bool| -> Vec<Span<'static>> {
        vec![
            Span::styled(k, dimmed),
            Span::styled(format!(" {} ", l), bold_text),
            Span::styled(
                format!("({})", v),
                if active { active_style } else { dimmed },
            ),
        ]
    };
    let pipe = || -> Span<'static> { Span::styled(" \u{2502} ", pipe_style) };

    let sort = app.worktree_sort_mode.label();

    let mut s: Vec<Span<'static>> = vec![Span::raw("  ")];
    s.extend(cmd("a".into(), "Add".into()));
    s.push(pipe());
    s.extend(cmd("o".into(), "PR".into()));
    s.push(pipe());
    s.extend(cmd("r".into(), "Remove".into()));
    s.push(pipe());
    s.extend(cmd("c".into(), "Close".into()));
    s.push(pipe());
    s.extend(cmd("R".into(), "Sweep".into()));
    s.push(pipe());
    s.extend(cmd("1-9".into(), "Jump".into()));
    s.push(pipe());
    s.extend(toggle("s".into(), "Sort".into(), sort.to_string(), true));
    if !app.worktree_filter_text.is_empty() {
        s.push(pipe());
        s.extend(cmd("/".into(), app.worktree_filter_text.clone()));
    } else {
        s.push(pipe());
        s.extend(cmd("/".into(), "Filter".into()));
    }
    s.push(pipe());
    s.extend(cmd("p".into(), "Project".into()));
    s.push(pipe());
    s.extend(cmd("Tab".into(), "Agents".into()));
    s.push(pipe());
    s.extend(cmd("q".into(), "Quit".into()));

    // Split footer: left commands, right-pinned help
    let right = Line::from(vec![
        Span::styled("?", dimmed),
        Span::styled(" Help ", bold_text),
    ]);
    let cols = Layout::horizontal([Constraint::Fill(1), Constraint::Length(7)]).split(area);

    f.render_widget(Paragraph::new(Line::from(s)), cols[0]);
    f.render_widget(Paragraph::new(right), cols[1]);
}
