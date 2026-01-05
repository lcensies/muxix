use anyhow::Result;
use console::{Style, Term};
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

fn render_markdown(input: &str) -> String {
    let mut output = String::new();
    let term_width = Term::stdout().size().1 as usize;
    let wrap_width = term_width.clamp(40, 100);

    let parser = Parser::new_ext(input, Options::all());

    let mut heading_level = 0;
    let mut in_code_block = false;
    let mut list_depth: usize = 0;
    let mut paragraph_buf = String::new();

    // Table state
    let mut in_table = false;
    let mut table_rows: Vec<Vec<String>> = Vec::new();
    let mut current_row: Vec<String> = Vec::new();
    let mut cell_buf = String::new();

    let h1 = Style::new().bold().cyan();
    let h2 = Style::new().bold().yellow();
    let h3 = Style::new().bold().green();
    let code_style = Style::new().dim();
    let bold_style = Style::new().bold();

    for event in parser {
        match event {
            // Table handling
            Event::Start(Tag::Table(_)) => {
                in_table = true;
                table_rows.clear();
            }
            Event::End(TagEnd::Table) => {
                // Render the table
                if !table_rows.is_empty() {
                    // Calculate column widths
                    let col_count = table_rows.iter().map(|r| r.len()).max().unwrap_or(0);
                    let mut col_widths: Vec<usize> = vec![0; col_count];
                    for row in &table_rows {
                        for (i, cell) in row.iter().enumerate() {
                            col_widths[i] = col_widths[i].max(cell.len());
                        }
                    }

                    // Render rows
                    for (row_idx, row) in table_rows.iter().enumerate() {
                        for (i, cell) in row.iter().enumerate() {
                            let width = col_widths.get(i).copied().unwrap_or(0);
                            if row_idx == 0 {
                                // Header row - bold
                                output.push_str(
                                    &bold_style.apply_to(format!("{:width$}", cell)).to_string(),
                                );
                            } else {
                                output.push_str(&format!("{:width$}", cell));
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

            Event::Start(Tag::Heading { level, .. }) => {
                heading_level = match level {
                    HeadingLevel::H1 => 1,
                    HeadingLevel::H2 => 2,
                    HeadingLevel::H3 => 3,
                    _ => 4,
                };
                output.push('\n');
            }
            Event::End(TagEnd::Heading(_)) => {
                let styled = match heading_level {
                    1 => h1.apply_to(&paragraph_buf).to_string(),
                    2 => h2.apply_to(&paragraph_buf).to_string(),
                    3 => h3.apply_to(&paragraph_buf).to_string(),
                    _ => bold_style.apply_to(&paragraph_buf).to_string(),
                };
                output.push_str(&styled);
                output.push_str("\n\n");
                paragraph_buf.clear();
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                if !paragraph_buf.is_empty() {
                    let wrapped = wrap(&paragraph_buf, wrap_width);
                    for line in wrapped {
                        output.push_str(&line);
                        output.push('\n');
                    }
                    output.push('\n');
                    paragraph_buf.clear();
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                in_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                output.push('\n');
            }
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
                paragraph_buf.clear();
            }
            Event::End(TagEnd::Item) => {
                // Skip empty items
                if !paragraph_buf.trim().is_empty() {
                    let base_indent = "  ".repeat(list_depth.saturating_sub(1));
                    let bullet = format!("{base_indent}• ");
                    let hang_indent = " ".repeat(bullet.len());
                    let item_width = wrap_width.saturating_sub(bullet.len());

                    if item_width > 10 {
                        let opts = WrapOptions::new(item_width).subsequent_indent(&hang_indent);
                        let wrapped = wrap(&paragraph_buf, opts);
                        for (i, line) in wrapped.iter().enumerate() {
                            if i == 0 {
                                output.push_str(&bullet);
                            }
                            output.push_str(line);
                            output.push('\n');
                        }
                    } else {
                        output.push_str(&bullet);
                        output.push_str(&paragraph_buf);
                        output.push('\n');
                    }
                }
                paragraph_buf.clear();
            }
            Event::Start(Tag::Strong | Tag::Emphasis | Tag::Link { .. }) => {}
            Event::End(TagEnd::Strong | TagEnd::Emphasis | TagEnd::Link) => {}
            Event::Code(text) => {
                if in_table {
                    cell_buf.push_str(&format!("`{text}`"));
                } else {
                    paragraph_buf.push_str(&code_style.apply_to(format!("`{text}`")).to_string());
                }
            }
            Event::Text(text) => {
                if in_table {
                    cell_buf.push_str(&text);
                } else if in_code_block {
                    for line in text.lines() {
                        output.push_str("    ");
                        output.push_str(&code_style.apply_to(line).to_string());
                        output.push('\n');
                    }
                } else {
                    paragraph_buf.push_str(&text);
                }
            }
            Event::SoftBreak => {
                if in_table {
                    cell_buf.push(' ');
                } else if !in_code_block {
                    paragraph_buf.push(' ');
                }
            }
            Event::HardBreak => {
                if in_table {
                    cell_buf.push(' ');
                } else {
                    paragraph_buf.push('\n');
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

    // Flush any remaining paragraph
    if !paragraph_buf.is_empty() {
        let wrapped = wrap(&paragraph_buf, wrap_width);
        for line in wrapped {
            output.push_str(&line);
            output.push('\n');
        }
    }

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
