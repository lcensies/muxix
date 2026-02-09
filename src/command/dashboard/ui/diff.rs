//! Diff view rendering (normal diff, patch mode, file list).

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, List, ListItem, Paragraph},
};

use super::super::diff::DiffView;
use super::theme::ThemePalette;

/// Render the diff view (replaces the entire dashboard).
pub fn render_diff_view(f: &mut Frame, diff: &mut DiffView, palette: &ThemePalette) {
    let area = f.area();

    // Layout: content area + footer
    let chunks = Layout::vertical([
        Constraint::Min(1),    // Content area
        Constraint::Length(1), // Footer
    ])
    .split(area);

    // Split content: File List (Right) + Diff (Left)
    // Only show file list if there are files to display
    let has_files = !diff.file_list.is_empty();
    let content_chunks = if has_files {
        Layout::horizontal([
            Constraint::Min(40),        // Diff content (takes remaining space)
            Constraint::Percentage(25), // File list sidebar
        ])
        .split(chunks[0])
    } else {
        // No files - use full width for diff
        Layout::horizontal([Constraint::Percentage(100)]).split(chunks[0])
    };

    let diff_area = content_chunks[0];
    let file_list_area = if has_files {
        Some(content_chunks[1])
    } else {
        None
    };

    // Update viewport height for scroll calculations (subtract 2 for borders)
    diff.viewport_height = diff_area.height.saturating_sub(2);

    if diff.patch_mode {
        // Patch mode with optional file list sidebar
        render_patch_mode(f, diff, diff_area, chunks[1], palette);
        if let Some(file_area) = file_list_area {
            render_file_list(f, diff, file_area, palette);
        }
    } else {
        // Normal diff mode with optional file list
        render_normal_diff(f, diff, diff_area, chunks[1], palette);
        if let Some(file_area) = file_list_area {
            render_file_list(f, diff, file_area, palette);
        }
    }
}

/// Determine which file is currently visible based on scroll position or current hunk.
fn get_current_file_index(diff: &DiffView) -> Option<usize> {
    if diff.file_list.is_empty() {
        return None;
    }

    // In patch mode, use the current hunk's filename
    if diff.patch_mode && !diff.hunks.is_empty() {
        let current_filename = &diff.hunks[diff.current_hunk].filename;
        return diff
            .file_list
            .iter()
            .position(|f| &f.filename == current_filename);
    }

    // Find the last file whose start_line is <= current scroll position
    let mut current_idx = 0;
    for (idx, file) in diff.file_list.iter().enumerate() {
        if file.start_line <= diff.scroll {
            current_idx = idx;
        } else {
            break;
        }
    }
    Some(current_idx)
}

/// Render the file list sidebar (full paths, directory dimmed, left-truncate if needed).
fn render_file_list(f: &mut Frame, diff: &DiffView, area: Rect, palette: &ThemePalette) {
    let current_file_idx = get_current_file_index(diff);

    let block = Block::bordered()
        .title(format!(" Files ({}) ", diff.file_list.len()))
        .title_style(Style::default().fg(Color::Cyan))
        .border_style(Style::default().fg(palette.dimmed));

    // Calculate available width (subtract borders)
    let inner_width = area.width.saturating_sub(2) as usize;

    let mut items: Vec<ListItem> = Vec::new();

    for (idx, file) in diff.file_list.iter().enumerate() {
        let is_current = current_file_idx == Some(idx);

        // Determine status indicator
        let (status_char, status_color) = if file.is_new {
            ("A", Color::Green)
        } else if file.lines_added == 0 && file.lines_removed > 0 {
            ("D", Color::Red)
        } else {
            ("M", Color::Yellow)
        };

        // Format stats
        let stats = match (file.lines_added, file.lines_removed) {
            (0, 0) => String::new(),
            (a, 0) => format!("+{}", a),
            (0, r) => format!("-{}", r),
            (a, r) => format!("+{} -{}", a, r),
        };

        // Calculate space for path: inner_width - status(2) - stats - min_padding(1)
        let stats_width = if stats.is_empty() { 0 } else { stats.len() + 1 };
        let path_max_width = inner_width.saturating_sub(2 + stats_width);

        // Split into directory and basename
        let (dir, basename) = match file.filename.rsplit_once('/') {
            Some((d, b)) => (Some(d), b),
            None => (None, file.filename.as_str()),
        };

        // Truncate path from left if needed
        let full_path_len = file.filename.len();
        let (display_dir, display_basename) = if full_path_len > path_max_width {
            // Need to truncate - prioritize showing basename
            if basename.len() >= path_max_width {
                // Even basename doesn't fit, truncate it
                let trunc_len = path_max_width.saturating_sub(1); // -1 for "..."
                (
                    None,
                    format!(
                        "...{}",
                        &basename[basename.len().saturating_sub(trunc_len)..]
                    ),
                )
            } else {
                // Truncate directory, keep full basename
                // path_max_width = "..." + dir_chars + "/" + basename
                // dir_chars = path_max_width - 1 (...) - 1 (/) - basename.len()
                let dir_chars = path_max_width.saturating_sub(2 + basename.len());
                match dir {
                    Some(d) if dir_chars > 0 => {
                        let start = d.len().saturating_sub(dir_chars);
                        let truncated = format!("...{}", &d[start..]);
                        (Some(truncated), basename.to_string())
                    }
                    Some(_) => (Some("...".to_string()), basename.to_string()),
                    None => (None, basename.to_string()),
                }
            }
        } else {
            (dir.map(|d| d.to_string()), basename.to_string())
        };

        // Calculate displayed path length
        let path_len = match &display_dir {
            Some(d) => d.len() + 1 + display_basename.len(), // dir/ + basename
            None => display_basename.len(),
        };

        // Calculate padding to right-align stats (minimum 1 space)
        // Total used: status(2) + path_len + padding + stats
        let padding = inner_width
            .saturating_sub(2) // status char + space
            .saturating_sub(path_len)
            .saturating_sub(stats.len())
            .max(1);

        // Build spans
        let mut spans = vec![Span::styled(
            format!("{} ", status_char),
            Style::default().fg(status_color),
        )];

        // Path with directory dimmed
        let basename_style = if is_current {
            Style::default()
                .fg(palette.text)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        if let Some(d) = display_dir {
            spans.push(Span::styled(
                format!("{}/", d),
                Style::default().fg(palette.dimmed),
            ));
        }
        spans.push(Span::styled(display_basename, basename_style));

        // Padding + stats (right-aligned)
        if !stats.is_empty() {
            spans.push(Span::raw(" ".repeat(padding)));
            // Color the stats
            if file.lines_added > 0 && file.lines_removed > 0 {
                spans.push(Span::styled(
                    format!("+{}", file.lines_added),
                    Style::default().fg(Color::Green),
                ));
                spans.push(Span::styled(
                    format!(" -{}", file.lines_removed),
                    Style::default().fg(Color::Red),
                ));
            } else if file.lines_added > 0 {
                spans.push(Span::styled(stats, Style::default().fg(Color::Green)));
            } else {
                spans.push(Span::styled(stats, Style::default().fg(Color::Red)));
            }
        }

        items.push(ListItem::new(Line::from(spans)));
    }

    let list = List::new(items).block(block);

    f.render_widget(list, area);
}

/// Render normal diff view (full diff with scroll).
fn render_normal_diff(
    f: &mut Frame,
    diff: &DiffView,
    content_area: Rect,
    footer_area: Rect,
    palette: &ThemePalette,
) {
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
        .border_style(Style::default().fg(palette.dimmed));

    // Calculate inner area (content area minus borders)
    let inner_height = content_area.height.saturating_sub(2) as usize;

    // Virtualize: slice only the visible lines from cached parsed_lines
    let max_start = diff.parsed_lines.len().saturating_sub(1);
    let start = diff.scroll.min(max_start);
    let end = (start + inner_height).min(diff.parsed_lines.len());
    let visible_lines: Vec<Line> = diff.parsed_lines[start..end].to_vec();
    let text = Text::from(visible_lines);

    // Render without scroll offset (already sliced to visible portion)
    let paragraph = Paragraph::new(text).block(block);

    f.render_widget(paragraph, content_area);

    // Footer with keybindings - show which diff type is active (toggle with d)
    let (wip_style, review_style) = if diff.is_branch_diff {
        (
            Style::default().fg(palette.dimmed),
            Style::default().fg(Color::Green),
        )
    } else {
        (
            Style::default().fg(Color::Green),
            Style::default().fg(palette.dimmed),
        )
    };

    let mut footer_spans = vec![
        Span::raw("  "),
        Span::styled("[Tab]", Style::default().fg(Color::Yellow)),
        Span::raw(" "),
        Span::styled("WIP", wip_style),
        Span::styled(" | ", Style::default().fg(palette.dimmed)),
        Span::styled("review", review_style),
        Span::raw("  "),
    ];

    // Show [a] patch option only for WIP mode with changes
    if !diff.is_branch_diff && (diff.lines_added > 0 || diff.lines_removed > 0) {
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

/// Render patch mode (hunk-by-hunk staging like git add -p).
fn render_patch_mode(
    f: &mut Frame,
    diff: &DiffView,
    content_area: Rect,
    footer_area: Rect,
    palette: &ThemePalette,
) {
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

    // Calculate inner area (content area minus borders)
    let inner_height = content_area.height.saturating_sub(2) as usize;

    // Virtualize: slice only the visible lines from cached parsed_lines
    let max_start = hunk.parsed_lines.len().saturating_sub(1);
    let start = diff.scroll.min(max_start);
    let end = (start + inner_height).min(hunk.parsed_lines.len());
    let visible_lines: Vec<Line> = hunk.parsed_lines[start..end].to_vec();
    let text = Text::from(visible_lines);

    // Render without scroll offset (already sliced to visible portion)
    let paragraph = Paragraph::new(text).block(block);

    f.render_widget(paragraph, content_area);

    // Footer: show comment input if in comment mode, otherwise show keybindings
    if let Some(ref input) = diff.comment_input {
        // Comment input mode - hints on left stay fixed, input on right
        let mut spans = vec![
            Span::styled("  [Enter]", Style::default().fg(Color::Green)),
            Span::raw(" send  "),
            Span::styled("[Esc]", Style::default().fg(Color::Red)),
            Span::raw(" cancel  "),
            Span::styled("| ", Style::default().fg(palette.dimmed)),
        ];

        if input.is_empty() {
            // Show cursor then placeholder when empty
            spans.push(Span::styled("|", Style::default().fg(palette.text)));
            spans.push(Span::styled(
                "Type your comment...",
                Style::default().fg(palette.dimmed),
            ));
        } else {
            spans.push(Span::raw(input));
            spans.push(Span::styled("|", Style::default().fg(palette.text)));
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
            Span::styled("[o]", Style::default().fg(Color::Cyan)),
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
