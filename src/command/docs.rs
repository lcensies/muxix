use anyhow::Result;
use console::{Style, Term, measure_text_width};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};
use textwrap::{Options as WrapOptions, wrap};

const README: &str = include_str!("../../README.md");

pub fn run() -> Result<()> {
    let rendered = render_markdown(README);

    if !std::io::stdout().is_terminal() {
        print!("{rendered}");
        return Ok(());
    }

    let pager = std::env::var("PAGER").unwrap_or_else(|_| "less -R".to_string());
    let mut parts = pager.split_whitespace();
    let cmd = parts.next().unwrap_or("less");
    let args: Vec<&str> = parts.collect();

    if let Ok(mut child) = Command::new(cmd).args(&args).stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(rendered.as_bytes());
        }
        let _ = child.wait();
    } else {
        print!("{rendered}");
    }

    Ok(())
}

/// Represents a segment of text with optional styling
#[derive(Clone)]
struct StyledSegment {
    text: String,
    bold: bool,
    italic: bool,
    code: bool,
    link_url: Option<String>,
}

impl StyledSegment {
    fn code(text: &str) -> Self {
        Self {
            text: text.to_string(),
            bold: false,
            italic: false,
            code: true,
            link_url: None,
        }
    }
}

/// Buffer for collecting content that will be wrapped together
struct TextBuffer {
    segments: Vec<StyledSegment>,
    bold: bool,
    italic: bool,
    link_url: Option<String>,
}

impl TextBuffer {
    fn new() -> Self {
        Self {
            segments: Vec::new(),
            bold: false,
            italic: false,
            link_url: None,
        }
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.segments.push(StyledSegment {
            text: text.to_string(),
            bold: self.bold,
            italic: self.italic,
            code: false,
            link_url: self.link_url.clone(),
        });
    }

    fn push_code(&mut self, text: &str) {
        self.segments.push(StyledSegment::code(text));
    }

    fn push_link_url(&mut self, url: &str) {
        if !url.starts_with('#') {
            self.segments.push(StyledSegment {
                text: format!(" ({})", url),
                bold: false,
                italic: false,
                code: false,
                link_url: Some(url.to_string()),
            });
        }
    }

    fn clear(&mut self) {
        self.segments.clear();
        self.bold = false;
        self.italic = false;
        self.link_url = None;
    }

    fn is_empty(&self) -> bool {
        self.segments.is_empty() || self.segments.iter().all(|s| s.text.trim().is_empty())
    }

    /// Get plain text for wrapping calculation
    fn plain_text(&self) -> String {
        self.segments
            .iter()
            .map(|s| {
                if s.code {
                    format!("`{}`", s.text)
                } else {
                    s.text.clone()
                }
            })
            .collect()
    }

    /// Render with styles applied
    fn render(&self, styles: &Styles) -> String {
        self.segments
            .iter()
            .map(|s| {
                let text = if s.code {
                    format!("`{}`", s.text)
                } else {
                    s.text.clone()
                };

                if s.code {
                    styles.code.apply_to(text).to_string()
                } else if s.link_url.is_some() {
                    styles.link.apply_to(text).to_string()
                } else if s.bold && s.italic {
                    styles.bold_italic.apply_to(text).to_string()
                } else if s.bold {
                    styles.bold.apply_to(text).to_string()
                } else if s.italic {
                    styles.italic.apply_to(text).to_string()
                } else {
                    text
                }
            })
            .collect()
    }
}

struct Styles {
    h1: Style,
    h2: Style,
    h3: Style,
    h4: Style,
    bold: Style,
    italic: Style,
    bold_italic: Style,
    code: Style,
    link: Style,
}

impl Default for Styles {
    fn default() -> Self {
        Self {
            h1: Style::new().bold().cyan(),
            h2: Style::new().bold().yellow(),
            h3: Style::new().bold().green(),
            h4: Style::new().bold(),
            bold: Style::new().bold(),
            italic: Style::new().italic(),
            bold_italic: Style::new().bold().italic(),
            code: Style::new().dim(),
            link: Style::new().blue().underlined(),
        }
    }
}

/// Wrap text while preserving ANSI codes
fn wrap_styled_text(text: &str, width: usize, subsequent_indent: &str) -> Vec<String> {
    // Wrap the plain text first
    let opts = WrapOptions::new(width).subsequent_indent(subsequent_indent);
    wrap(text, opts)
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}

fn render_markdown(input: &str) -> String {
    let mut output = String::new();
    let term_width = Term::stdout().size().1 as usize;
    let wrap_width = term_width.clamp(40, 100);

    let parser = Parser::new_ext(input, Options::all());
    let styles = Styles::default();

    // State
    let mut text_buf = TextBuffer::new();
    let mut list_item_lines: Vec<String> = Vec::new();
    let mut list_depth: usize = 0;
    let mut in_code_block = false;
    let mut heading_level = 0;

    // Table state
    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut cell_buf = String::new();

    // Helper to flush text buffer to output or list
    let flush_text = |buf: &mut TextBuffer,
                      output: &mut String,
                      list_lines: &mut Vec<String>,
                      list_depth: usize,
                      wrap_width: usize,
                      styles: &Styles| {
        if buf.is_empty() {
            buf.clear();
            return;
        }

        let plain = buf.plain_text();
        let indent = if list_depth > 0 {
            "  ".repeat(list_depth)
        } else {
            String::new()
        };
        let effective_width = wrap_width.saturating_sub(indent.len());

        if effective_width < 20 {
            // Too narrow, just output as-is
            let rendered = buf.render(styles);
            if list_depth > 0 {
                list_lines.push(rendered);
            } else {
                output.push_str(&rendered);
                output.push('\n');
            }
        } else {
            let wrapped = wrap_styled_text(&plain, effective_width, "");
            for line in wrapped {
                if list_depth > 0 {
                    list_lines.push(line);
                } else {
                    output.push_str(&line);
                    output.push('\n');
                }
            }
        }
        buf.clear();
    };

    for event in parser {
        match event {
            // === Table handling ===
            Event::Start(Tag::Table(_)) => {
                in_table = true;
                table_rows.clear();
            }
            Event::End(TagEnd::Table) => {
                render_table(&table_rows, &mut output, wrap_width, &styles);
                in_table = false;
                table_rows.clear();
            }
            Event::Start(Tag::TableHead | Tag::TableRow) => {
                current_row.clear();
            }
            Event::End(TagEnd::TableHead | TagEnd::TableRow) => {
                table_rows.push(current_row.clone());
                current_row.clear();
            }
            Event::Start(Tag::TableCell) => {
                cell_buf.clear();
            }
            Event::End(TagEnd::TableCell) => {
                current_row.push(cell_buf.trim().to_string());
                cell_buf.clear();
            }

            // === Headings ===
            Event::Start(Tag::Heading { level, .. }) => {
                heading_level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    _ => 4,
                };
                output.push('\n');
                text_buf.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                let plain = text_buf.plain_text();
                let styled = match heading_level {
                    1 => styles.h1.apply_to(&plain).to_string(),
                    2 => styles.h2.apply_to(&plain).to_string(),
                    3 => styles.h3.apply_to(&plain).to_string(),
                    _ => styles.h4.apply_to(&plain).to_string(),
                };
                output.push_str(&styled);
                output.push_str("\n\n");
                text_buf.clear();
            }

            // === Paragraphs ===
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                flush_text(
                    &mut text_buf,
                    &mut output,
                    &mut list_item_lines,
                    list_depth,
                    wrap_width,
                    &styles,
                );
                if list_depth == 0 {
                    output.push('\n');
                }
            }

            // === Code blocks ===
            Event::Start(Tag::CodeBlock(_)) => {
                // Flush any pending text first
                flush_text(
                    &mut text_buf,
                    &mut output,
                    &mut list_item_lines,
                    list_depth,
                    wrap_width,
                    &styles,
                );
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                if list_depth == 0 {
                    output.push('\n');
                }
            }

            // === Lists ===
            Event::Start(Tag::List(_)) => {
                list_depth += 1;
            }
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                if list_depth == 0 {
                    output.push('\n');
                }
            }
            Event::Start(Tag::Item) => {
                list_item_lines.clear();
                text_buf.clear();
            }
            Event::End(TagEnd::Item) => {
                // Flush any remaining text
                flush_text(
                    &mut text_buf,
                    &mut output,
                    &mut list_item_lines,
                    list_depth,
                    wrap_width,
                    &styles,
                );

                // Render the list item
                if !list_item_lines.is_empty() {
                    let base_indent = "  ".repeat(list_depth.saturating_sub(1));
                    let bullet = format!("{base_indent}• ");
                    let hang_indent = " ".repeat(bullet.len());

                    for (i, line) in list_item_lines.iter().enumerate() {
                        if i == 0 {
                            output.push_str(&bullet);
                        } else {
                            output.push_str(&hang_indent);
                        }
                        output.push_str(line);
                        output.push('\n');
                    }
                }
                list_item_lines.clear();
            }

            // === Inline styles ===
            Event::Start(Tag::Strong) => {
                text_buf.bold = true;
            }
            Event::End(TagEnd::Strong) => {
                text_buf.bold = false;
            }
            Event::Start(Tag::Emphasis) => {
                text_buf.italic = true;
            }
            Event::End(TagEnd::Emphasis) => {
                text_buf.italic = false;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                text_buf.link_url = Some(dest_url.to_string());
            }
            Event::End(TagEnd::Link) => {
                if let Some(url) = text_buf.link_url.take() {
                    text_buf.push_link_url(&url);
                }
            }

            // === Content ===
            Event::Code(text) => {
                if in_table {
                    cell_buf.push_str(&format!("`{text}`"));
                } else {
                    text_buf.push_code(&text);
                }
            }
            Event::Text(text) => {
                if in_table {
                    cell_buf.push_str(&text);
                } else if in_code_block {
                    // Code blocks go directly to output/list, preserving order
                    for line in text.lines() {
                        let styled = format!("    {}", styles.code.apply_to(line));
                        if list_depth > 0 {
                            list_item_lines.push(styled);
                        } else {
                            output.push_str(&styled);
                            output.push('\n');
                        }
                    }
                } else {
                    text_buf.push_text(&text);
                }
            }
            Event::SoftBreak => {
                if in_table {
                    cell_buf.push(' ');
                } else if !in_code_block {
                    text_buf.push_text(" ");
                }
            }
            Event::HardBreak => {
                if in_table {
                    cell_buf.push(' ');
                } else if !in_code_block {
                    // Flush current line and start new one
                    flush_text(
                        &mut text_buf,
                        &mut output,
                        &mut list_item_lines,
                        list_depth,
                        wrap_width,
                        &styles,
                    );
                }
            }
            Event::Rule => {
                output.push_str(&"─".repeat(wrap_width));
                output.push_str("\n\n");
            }
            Event::Html(_) => {}
            _ => {}
        }
    }

    // Flush any remaining content
    flush_text(
        &mut text_buf,
        &mut output,
        &mut list_item_lines,
        list_depth,
        wrap_width,
        &styles,
    );

    // Clean up excessive newlines
    let mut result = String::new();
    let mut newline_count = 0;
    for c in output.chars() {
        if c == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                result.push(c);
            }
        } else {
            newline_count = 0;
            result.push(c);
        }
    }

    result.trim().to_string() + "\n"
}

fn render_table(rows: &[Vec<String>], output: &mut String, max_width: usize, styles: &Styles) {
    if rows.is_empty() {
        return;
    }

    let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if col_count == 0 {
        return;
    }

    // Calculate column widths based on content
    let mut col_widths: Vec<usize> = vec![0; col_count];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            col_widths[i] = col_widths[i].max(measure_text_width(cell));
        }
    }

    // Cap column widths if table is too wide
    let total_width: usize = col_widths.iter().sum::<usize>() + (col_count - 1) * 2;
    if total_width > max_width {
        // Shrink columns proportionally, but keep minimum of 10
        let scale = max_width as f64 / total_width as f64;
        for w in &mut col_widths {
            *w = ((*w as f64 * scale) as usize).max(10);
        }
    }

    // Render rows
    for (row_idx, row) in rows.iter().enumerate() {
        for (i, cell) in row.iter().enumerate() {
            let width = col_widths.get(i).copied().unwrap_or(0);
            let cell_text = if measure_text_width(cell) > width {
                // Truncate with ellipsis
                let mut truncated = String::new();
                for (i, c) in cell.chars().enumerate() {
                    if i + 1 > width.saturating_sub(1) {
                        truncated.push('…');
                        break;
                    }
                    truncated.push(c);
                }
                truncated
            } else {
                cell.clone()
            };

            let padded = format!("{:width$}", cell_text, width = width);
            if row_idx == 0 {
                output.push_str(&styles.bold.apply_to(&padded).to_string());
            } else {
                output.push_str(&padded);
            }
            if i < row.len() - 1 {
                output.push_str("  ");
            }
        }
        output.push('\n');

        // Add separator after header
        if row_idx == 0 {
            for (i, &width) in col_widths.iter().enumerate() {
                output.push_str(&"─".repeat(width));
                if i < col_widths.len() - 1 {
                    output.push_str("  ");
                }
            }
            output.push('\n');
        }
    }
    output.push('\n');
}
