//! ANSI escape sequence handling utilities.

use ansi_to_tui::IntoText;
use ratatui::text::Line;

/// Strip ANSI escape sequences from a string
pub fn strip_ansi_escapes(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip escape sequence
            if chars.peek() == Some(&'[') {
                chars.next(); // consume '['
                // Skip until we hit a letter (the terminator)
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Parse ANSI-escaped content into a vector of Lines for efficient rendering.
/// This is cached to avoid re-parsing on every frame.
pub fn parse_ansi_to_lines(content: &str) -> Vec<Line<'static>> {
    content
        .into_text()
        .map(|text| text.lines)
        .unwrap_or_else(|_| {
            // Fallback: split by newlines and create raw lines
            content.lines().map(|s| Line::raw(s.to_string())).collect()
        })
}
