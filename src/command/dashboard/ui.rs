//! TUI rendering logic for the dashboard.

use ansi_to_tui::IntoText;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Cell, Paragraph, Row, Table},
};
use std::collections::{BTreeMap, HashSet};

use crate::git::GitStatus;

use super::app::{App, DiffView, ViewMode};

/// Braille spinner frames for subtle loading animation
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

/// Number of spinner frames (used by event loop to wrap frame counter)
pub const SPINNER_FRAME_COUNT: u8 = SPINNER_FRAMES.len() as u8;

pub fn ui(f: &mut Frame, app: &mut App) {
    // Render either dashboard or diff view based on view mode
    match &mut app.view_mode {
        ViewMode::Dashboard => render_dashboard(f, app),
        ViewMode::Diff(diff_view) => render_diff_view(f, diff_view),
    }
}

fn render_dashboard(f: &mut Frame, app: &mut App) {
    let area = f.area();

    // Layout: table (top), preview (bottom), footer
    let chunks = Layout::vertical([
        Constraint::Percentage(40), // Table (top half)
        Constraint::Min(5),         // Preview (bottom half, at least 5 lines)
        Constraint::Length(1),      // Footer
    ])
    .split(area);

    // Table
    render_table(f, app, chunks[0]);

    // Preview
    render_preview(f, app, chunks[1]);

    // Footer - show different help based on mode
    let footer_text = if app.input_mode {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  INPUT MODE",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" - Type to send keys to agent  "),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::raw(" exit"),
        ]))
    } else {
        let mut spans = vec![
            Span::styled("  [i]", Style::default().fg(Color::Green)),
            Span::raw(" input  "),
            Span::styled("[d]", Style::default().fg(Color::Yellow)),
            Span::raw(" diff  "),
            Span::styled("[1-9]", Style::default().fg(Color::Yellow)),
            Span::raw(" jump  "),
            Span::styled("[p]", Style::default().fg(Color::Cyan)),
            Span::raw(" peek  "),
            Span::styled("[s]", Style::default().fg(Color::Cyan)),
            Span::raw(" sort: "),
            Span::styled(app.sort_mode.label(), Style::default().fg(Color::Green)),
            Span::raw("  "),
            Span::styled("[f]", Style::default().fg(Color::Cyan)),
            Span::raw(" filter: "),
        ];

        if app.hide_stale {
            spans.push(Span::styled(
                "hiding stale",
                Style::default().fg(Color::Yellow),
            ));
        } else {
            spans.push(Span::styled("all", Style::default().fg(Color::DarkGray)));
        }

        spans.extend(vec![
            Span::raw("  "),
            Span::styled("[Enter]", Style::default().fg(Color::Cyan)),
            Span::raw(" go  "),
            Span::styled("[q]", Style::default().fg(Color::Cyan)),
            Span::raw(" quit"),
        ]);

        Paragraph::new(Line::from(spans))
    };
    f.render_widget(footer_text, chunks[2]);
}

/// Format git status for the Git column: base branch, diff stats, then indicators
/// Format: "→branch +N -M 󰏫 +X -Y 󰀪 ↑A ↓B"
/// When there are uncommitted changes that differ from total, branch totals are dimmed
fn format_git_status(status: Option<&GitStatus>, spinner_frame: u8) -> Vec<(String, Style)> {
    if let Some(status) = status {
        let mut spans: Vec<(String, Style)> = Vec::new();
        let has_uncommitted =
            status.uncommitted_added > 0 || status.uncommitted_removed > 0 || status.is_dirty;

        // Check if uncommitted equals total (all changes are uncommitted, nothing committed yet)
        let all_uncommitted = status.uncommitted_added == status.lines_added
            && status.uncommitted_removed == status.lines_removed;

        // Base branch (dimmed) - only show if not default (main/master)
        if !status.base_branch.is_empty()
            && status.base_branch != "main"
            && status.base_branch != "master"
        {
            spans.push((
                format!("→{}", status.base_branch),
                Style::default().fg(Color::DarkGray),
            ));
        }

        // Always dim branch totals (historical), always bright uncommitted (active work)
        // - Clean: dim branch totals only
        // - All uncommitted: icon + bright uncommitted only
        // - Mixed: dim branch totals + icon + bright uncommitted
        if has_uncommitted && all_uncommitted {
            // All changes are uncommitted - show icon + bright numbers only
            if !spans.is_empty() {
                spans.push((" ".to_string(), Style::default()));
            }
            spans.push(("\u{f03eb}".to_string(), Style::default().fg(Color::Magenta)));

            if status.uncommitted_added > 0 {
                spans.push((" ".to_string(), Style::default()));
                spans.push((
                    format!("+{}", status.uncommitted_added),
                    Style::default().fg(Color::Green),
                ));
            }
            if status.uncommitted_removed > 0 {
                spans.push((" ".to_string(), Style::default()));
                spans.push((
                    format!("-{}", status.uncommitted_removed),
                    Style::default().fg(Color::Red),
                ));
            }
        } else {
            // Either clean or mixed - show dim branch totals
            if status.lines_added > 0 {
                if !spans.is_empty() {
                    spans.push((" ".to_string(), Style::default()));
                }
                spans.push((
                    format!("+{}", status.lines_added),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::DIM),
                ));
            }
            if status.lines_removed > 0 {
                if !spans.is_empty() {
                    spans.push((" ".to_string(), Style::default()));
                }
                spans.push((
                    format!("-{}", status.lines_removed),
                    Style::default().fg(Color::Red).add_modifier(Modifier::DIM),
                ));
            }

            // If there are uncommitted changes, show icon + bright uncommitted
            if has_uncommitted {
                if !spans.is_empty() {
                    spans.push((" ".to_string(), Style::default()));
                }
                spans.push(("\u{f03eb}".to_string(), Style::default().fg(Color::Magenta)));

                if status.uncommitted_added > 0 {
                    spans.push((" ".to_string(), Style::default()));
                    spans.push((
                        format!("+{}", status.uncommitted_added),
                        Style::default().fg(Color::Green),
                    ));
                }
                if status.uncommitted_removed > 0 {
                    spans.push((" ".to_string(), Style::default()));
                    spans.push((
                        format!("-{}", status.uncommitted_removed),
                        Style::default().fg(Color::Red),
                    ));
                }
            }
        }

        // Conflict indicator
        if status.has_conflict {
            if !spans.is_empty() {
                spans.push((" ".to_string(), Style::default()));
            }
            spans.push(("\u{f002a}".to_string(), Style::default().fg(Color::Red)));
        }

        // Ahead/behind upstream
        if status.ahead > 0 {
            if !spans.is_empty() {
                spans.push((" ".to_string(), Style::default()));
            }
            spans.push((
                format!("↑{}", status.ahead),
                Style::default().fg(Color::Blue),
            ));
        }
        if status.behind > 0 {
            if !spans.is_empty() {
                spans.push((" ".to_string(), Style::default()));
            }
            spans.push((
                format!("↓{}", status.behind),
                Style::default().fg(Color::Yellow),
            ));
        }

        if spans.is_empty() {
            vec![("-".to_string(), Style::default().fg(Color::DarkGray))]
        } else {
            spans
        }
    } else {
        // No status yet - show spinner
        let frame = SPINNER_FRAMES[spinner_frame as usize % SPINNER_FRAMES.len()];
        vec![(frame.to_string(), Style::default().fg(Color::DarkGray))]
    }
}

fn render_table(f: &mut Frame, app: &mut App, area: Rect) {
    let header_cells = ["#", "Project", "Worktree", "Git", "Status", "Time", "Title"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Cyan).bold()));
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
            let worktree_display = format!("{}{}", worktree_name, pane_suffix);
            let title = agent
                .pane_title
                .as_ref()
                .map(|t| t.strip_prefix("✳ ").unwrap_or(t).to_string())
                .unwrap_or_default();
            let (status_text, status_color) = app.get_status_display(agent);
            let duration = app
                .get_elapsed(agent)
                .map(|d| app.format_duration(d))
                .unwrap_or_else(|| "-".to_string());

            // Get git status for this worktree (may be None if not yet fetched)
            let git_status = app.git_statuses.get(&agent.path);
            let git_spans = format_git_status(git_status, app.spinner_frame);

            (
                jump_key,
                project,
                worktree_display,
                is_main,
                git_spans,
                status_text,
                status_color,
                duration,
                title,
            )
        })
        .collect();

    // Calculate max project name width (with padding, capped)
    let max_project_width = row_data
        .iter()
        .map(|(_, project, _, _, _, _, _, _, _)| project.len())
        .max()
        .unwrap_or(5)
        .clamp(5, 20) // min 5, max 20
        + 2; // padding

    // Calculate max worktree name width (with padding)
    // Use at least 8 to fit the "Worktree" header
    let max_worktree_width = row_data
        .iter()
        .map(|(_, _, worktree_display, _, _, _, _, _, _)| worktree_display.len())
        .max()
        .unwrap_or(8)
        .max(8) // min 8 (header width)
        + 1; // padding

    // Calculate max git status width (sum of all span character counts)
    // Use chars().count() instead of len() because Nerd Font icons are multi-byte
    let max_git_width = row_data
        .iter()
        .map(|(_, _, _, _, git_spans, _, _, _, _)| {
            git_spans
                .iter()
                .map(|(text, _)| text.chars().count())
                .sum::<usize>()
        })
        .max()
        .unwrap_or(4)
        .clamp(4, 30) // min 4, max 30 (increased for base branch)
        + 1; // padding

    let rows: Vec<Row> = row_data
        .into_iter()
        .map(
            |(
                jump_key,
                project,
                worktree_display,
                is_main,
                git_spans,
                status_text,
                status_color,
                duration,
                title,
            )| {
                let worktree_style = if is_main {
                    Style::default().fg(Color::DarkGray)
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
                Row::new(vec![
                    Cell::from(jump_key).style(Style::default().fg(Color::Yellow)),
                    Cell::from(project),
                    Cell::from(worktree_display).style(worktree_style),
                    Cell::from(git_line),
                    Cell::from(status_text).style(Style::default().fg(status_color)),
                    Cell::from(duration),
                    Cell::from(title),
                ])
            },
        )
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(2),                         // #: jump key
            Constraint::Length(max_project_width as u16),  // Project: auto-sized
            Constraint::Length(max_worktree_width as u16), // Worktree: auto-sized
            Constraint::Length(max_git_width as u16),      // Git: auto-sized
            Constraint::Length(8),                         // Status: fixed (icons)
            Constraint::Length(10),                        // Time: HH:MM:SS + padding
            Constraint::Fill(1),                           // Title: takes remaining space
        ],
    )
    .header(header)
    .block(Block::default())
    .row_highlight_style(Style::default().bg(Color::Rgb(50, 50, 55)))
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
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Style::default().fg(Color::Green),
        )
    } else if let Some(agent) = selected_agent {
        let worktree_name = app.extract_worktree_name(agent).0;
        (
            format!(" Preview: {} ", worktree_name),
            Style::default().fg(Color::Cyan),
            Style::default().fg(Color::DarkGray),
        )
    } else {
        (
            " Preview ".to_string(),
            Style::default().fg(Color::Cyan),
            Style::default().fg(Color::DarkGray),
        )
    };

    let block = Block::bordered()
        .title(title)
        .title_style(title_style)
        .border_style(border_style);

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
                        // Fallback to plain text if ANSI parsing fails
                        let count = trimmed.lines().count() as u16;
                        (Text::raw(trimmed), count)
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

/// Render the diff view (replaces the entire dashboard)
fn render_diff_view(f: &mut Frame, diff: &mut DiffView) {
    let area = f.area();

    // Layout: content area + footer
    let chunks = Layout::vertical([
        Constraint::Min(1),    // Content area
        Constraint::Length(1), // Footer
    ])
    .split(area);

    // Update viewport height for scroll calculations (subtract 2 for borders)
    diff.viewport_height = chunks[0].height.saturating_sub(2);

    if diff.patch_mode {
        // Patch mode: show current hunk
        render_patch_mode(f, diff, chunks[0], chunks[1]);
    } else {
        // Normal diff mode
        render_normal_diff(f, diff, chunks[0], chunks[1]);
    }
}

/// Render normal diff view (full diff with scroll)
fn render_normal_diff(f: &mut Frame, diff: &DiffView, content_area: Rect, footer_area: Rect) {
    // Create block with title including diff stats
    let title = Line::from(vec![
        Span::styled(
            format!(" {} ", diff.title),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("+{}", diff.lines_added),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" "),
        Span::styled(
            format!("-{}", diff.lines_removed),
            Style::default().fg(Color::Red),
        ),
        Span::raw(" "),
    ]);
    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(Color::DarkGray));

    // Parse ANSI colors from diff content
    let text = diff
        .content
        .as_str()
        .into_text()
        .unwrap_or_else(|_| Text::raw(&diff.content));

    // Render scrollable paragraph (cast scroll to u16, clamping for safety)
    let scroll_u16 = diff.scroll.min(u16::MAX as usize) as u16;
    let paragraph = Paragraph::new(text).block(block).scroll((scroll_u16, 0));

    f.render_widget(paragraph, content_area);

    // Footer with keybindings - show which diff type is active (toggle with d)
    let (wip_style, review_style) = if diff.is_branch_diff {
        (
            Style::default().fg(Color::DarkGray),
            Style::default().fg(Color::Green),
        )
    } else {
        (
            Style::default().fg(Color::Green),
            Style::default().fg(Color::DarkGray),
        )
    };

    let mut footer_spans = vec![
        Span::raw("  "),
        Span::styled("[Tab]", Style::default().fg(Color::Yellow)),
        Span::raw(" "),
        Span::styled("WIP", wip_style),
        Span::styled(" | ", Style::default().fg(Color::DarkGray)),
        Span::styled("review", review_style),
        Span::raw("  "),
    ];

    // Show [a] patch option only for WIP mode with hunks
    if !diff.is_branch_diff && !diff.hunks.is_empty() {
        footer_spans.push(Span::styled("[a]", Style::default().fg(Color::Magenta)));
        footer_spans.push(Span::raw(" patch  "));
    }

    footer_spans.extend(vec![
        Span::styled("[j/k]", Style::default().fg(Color::Cyan)),
        Span::raw(" scroll  "),
        Span::styled("[c]", Style::default().fg(Color::Green)),
        Span::raw(" commit  "),
        Span::styled("[m]", Style::default().fg(Color::Yellow)),
        Span::raw(" merge  "),
        Span::styled("[q]", Style::default().fg(Color::Cyan)),
        Span::raw(" close"),
    ]);

    let footer = Paragraph::new(Line::from(footer_spans));
    f.render_widget(footer, footer_area);
}

/// Render patch mode (hunk-by-hunk staging like git add -p)
fn render_patch_mode(f: &mut Frame, diff: &DiffView, content_area: Rect, footer_area: Rect) {
    let hunk = &diff.hunks[diff.current_hunk];

    // Title shows filename and hunk progress
    let title = Line::from(vec![
        Span::styled(
            " PATCH ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            &hunk.filename,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!(
                "[{}/{}]",
                diff.hunks_processed + diff.current_hunk + 1,
                diff.hunks_total
            ),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw(" "),
        Span::styled(
            format!("+{}", hunk.lines_added),
            Style::default().fg(Color::Green),
        ),
        Span::raw(" "),
        Span::styled(
            format!("-{}", hunk.lines_removed),
            Style::default().fg(Color::Red),
        ),
        Span::raw(" "),
    ]);

    let block = Block::bordered()
        .title(title)
        .border_style(Style::default().fg(Color::Magenta));

    // Use delta-rendered content with ANSI colors parsed
    let text = hunk
        .rendered_content
        .as_str()
        .into_text()
        .unwrap_or_else(|_| Text::raw(&hunk.rendered_content));

    // Render scrollable paragraph
    let scroll_u16 = diff.scroll.min(u16::MAX as usize) as u16;
    let paragraph = Paragraph::new(text).block(block).scroll((scroll_u16, 0));

    f.render_widget(paragraph, content_area);

    // Footer: show comment input if in comment mode, otherwise show keybindings
    if let Some(ref input) = diff.comment_input {
        // Comment input mode - hints on left stay fixed, input on right
        let mut spans = vec![
            Span::styled("  [Enter]", Style::default().fg(Color::Green)),
            Span::raw(" send  "),
            Span::styled("[Esc]", Style::default().fg(Color::Red)),
            Span::raw(" cancel  "),
            Span::styled("│ ", Style::default().fg(Color::DarkGray)),
        ];

        if input.is_empty() {
            // Show cursor then placeholder when empty
            spans.push(Span::styled("▌", Style::default().fg(Color::White)));
            spans.push(Span::styled(
                "Type your comment...",
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::raw(input));
            spans.push(Span::styled("▌", Style::default().fg(Color::White)));
        }

        let footer = Paragraph::new(Line::from(spans));
        f.render_widget(footer, footer_area);
    } else {
        // Normal patch mode keybindings
        let mut footer_spans = vec![
            Span::raw("  "),
            Span::styled("[y]", Style::default().fg(Color::Green)),
            Span::raw(" stage  "),
            Span::styled("[n]", Style::default().fg(Color::Red)),
            Span::raw(" skip  "),
        ];

        // Show undo option if there are staged hunks
        if !diff.staged_hunks.is_empty() {
            footer_spans.push(Span::styled("[u]", Style::default().fg(Color::Magenta)));
            footer_spans.push(Span::raw(" undo  "));
        }

        footer_spans.extend(vec![
            Span::styled("[s]", Style::default().fg(Color::Yellow)),
            Span::raw(" split  "),
            Span::styled("[c]", Style::default().fg(Color::Cyan)),
            Span::raw(" comment  "),
            Span::styled("[j/k]", Style::default().fg(Color::Cyan)),
            Span::raw(" nav  "),
            Span::styled("[q]", Style::default().fg(Color::Cyan)),
            Span::raw(" quit"),
        ]);

        let footer = Paragraph::new(Line::from(footer_spans));
        f.render_widget(footer, footer_area);
    }
}
