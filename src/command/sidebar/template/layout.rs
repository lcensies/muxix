//! Layout solver: turns a parsed token line and a RowContext into styled spans.

use ratatui::text::Span;

use super::context::RowContext;
use super::parser::{Token, TokenId};

/// Render a line of tokens into spans, fitting within `width` columns.
///
/// The algorithm:
/// 1. Split at `{fill}` if present.
/// 2. Right segment: each token renders at natural width.
/// 3. Left segment: fixed tokens at natural width; first flex token absorbs slack.
/// 4. If total exceeds width: drop right-segment field tokens in reverse order.
/// 5. If still exceeding: truncate leftmost flex token with ellipsis.
pub fn render_line(ctx: &RowContext, tokens: &[Token], width: usize) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }

    let fill_pos = tokens.iter().position(|t| matches!(t, Token::Fill));
    let (left_tokens, right_tokens) = match fill_pos {
        Some(pos) => (&tokens[..pos], &tokens[pos + 1..]),
        None => (tokens, &[][..]),
    };

    // Compute natural widths for all tokens
    let left_info: Vec<TokenInfo> = left_tokens.iter().map(|t| TokenInfo::new(t, ctx)).collect();
    let right_info: Vec<TokenInfo> = right_tokens
        .iter()
        .map(|t| TokenInfo::new(t, ctx))
        .collect();
    let left_info = collapse_empty_fields(left_info);
    let right_info = collapse_empty_fields(right_info);

    let right_width: usize = right_info.iter().map(|i| i.natural_width).sum();
    let left_fixed_width: usize = left_info
        .iter()
        .filter(|i| !i.is_flex)
        .map(|i| i.natural_width)
        .sum();

    let mut available = width.saturating_sub(right_width + left_fixed_width);

    // If available is negative, try dropping right-segment field tokens
    if available == 0 && right_width > 0 && right_width + left_fixed_width > width {
        let mut dropped_right_width = right_width;
        let mut right_kept: Vec<&TokenInfo> = right_info.iter().collect();

        // Drop field tokens (not literals) from the right in reverse order
        while let Some(last) = right_kept.last() {
            if last.is_field {
                dropped_right_width -= last.natural_width;
                right_kept.pop();
                // Also drop any trailing literals that follow the dropped field
                while let Some(last) = right_kept.last() {
                    if !last.is_field {
                        dropped_right_width -= last.natural_width;
                        right_kept.pop();
                    } else {
                        break;
                    }
                }
                available = width.saturating_sub(dropped_right_width + left_fixed_width);
                if available > 0 || dropped_right_width + left_fixed_width <= width {
                    break;
                }
            } else {
                break;
            }
        }

        // Rebuild right_info from kept tokens
        let right_info: Vec<TokenInfo> = right_kept.into_iter().cloned().collect();
        return render_with_layout(ctx, &left_info, &right_info, width, available);
    }

    render_with_layout(ctx, &left_info, &right_info, width, available)
}

/// Check whether a line would be empty after token resolution.
///
/// A line is empty when all variable-text tokens resolve to empty strings
/// and the line contains no non-whitespace literals.
pub fn is_empty_line(tokens: &[Token], ctx: &RowContext) -> bool {
    let mut has_content = false;
    for token in tokens {
        match token {
            Token::Literal(s) => {
                if s.trim().is_empty() {
                    continue;
                }
                // Non-whitespace literal means the line is not empty
                return false;
            }
            Token::Fill => {}
            Token::Field(id) => {
                let text = ctx.resolve(*id);
                if !text.is_empty() {
                    has_content = true;
                }
            }
        }
    }
    !has_content
}

#[derive(Clone)]
struct TokenInfo {
    token: Token,
    natural_width: usize,
    is_flex: bool,
    is_field: bool,
}

impl TokenInfo {
    fn new(token: &Token, ctx: &RowContext) -> Self {
        match token {
            Token::Literal(s) => Self {
                token: Token::Literal(s.clone()),
                natural_width: display_width(s),
                is_flex: false,
                is_field: false,
            },
            Token::Fill => Self {
                token: Token::Fill,
                natural_width: 0,
                is_flex: false,
                is_field: false,
            },
            Token::Field(id) => Self {
                token: Token::Field(*id),
                natural_width: ctx.natural_width(*id),
                is_flex: id.is_flex(),
                is_field: true,
            },
        }
    }
}

/// Drop a single adjacent whitespace literal next to any field that resolves
/// to empty, so optional fields like `{pane_suffix}` don't leave dangling
/// joiner spaces in the output.
fn collapse_empty_fields(infos: Vec<TokenInfo>) -> Vec<TokenInfo> {
    let mut keep = vec![true; infos.len()];
    for i in 0..infos.len() {
        let is_empty_field =
            matches!(infos[i].token, Token::Field(_)) && infos[i].natural_width == 0;
        if !is_empty_field {
            continue;
        }
        // Prefer dropping the following whitespace literal, fall back to the preceding one.
        if i + 1 < infos.len() && keep[i + 1] && is_whitespace_literal(&infos[i + 1]) {
            keep[i + 1] = false;
        } else if i > 0 && keep[i - 1] && is_whitespace_literal(&infos[i - 1]) {
            keep[i - 1] = false;
        }
    }
    infos
        .into_iter()
        .zip(keep)
        .filter_map(|(info, k)| if k { Some(info) } else { None })
        .collect()
}

fn is_whitespace_literal(info: &TokenInfo) -> bool {
    if let Token::Literal(s) = &info.token {
        !s.is_empty() && s.chars().all(|c| c.is_whitespace())
    } else {
        false
    }
}

fn render_with_layout(
    ctx: &RowContext,
    left: &[TokenInfo],
    right: &[TokenInfo],
    width: usize,
    mut available: usize,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut used_width = 0;
    let mut first_flex_assigned = false;
    let mut slack: usize = 0;

    // Render left segment
    for info in left {
        match &info.token {
            Token::Literal(s) => {
                spans.push(Span::raw(s.clone()));
                used_width += info.natural_width;
            }
            Token::Fill => {}
            Token::Field(id) => {
                if info.is_flex && !first_flex_assigned {
                    // First flex token: truncate if natural exceeds slack, otherwise
                    // render at natural width and emit the leftover as a fill-space
                    // span between left and right segments (handled after the loop).
                    let allocated = available;
                    if *id == TokenId::StatusIcon {
                        for (text, style) in &ctx.status_icon_spans {
                            spans.push(Span::styled(text.clone(), *style));
                            used_width += display_width(text);
                        }
                    } else {
                        let text = ctx.resolve(*id);
                        let rendered = if info.natural_width > allocated && allocated > 0 {
                            truncate_with_ellipsis(&text, allocated)
                        } else if allocated == 0 {
                            String::new()
                        } else {
                            text
                        };
                        let rendered_width = display_width(&rendered);
                        spans.push(Span::styled(rendered, ctx.intrinsic_style(*id)));
                        used_width += rendered_width;

                        if rendered_width < allocated {
                            slack = allocated - rendered_width;
                        }
                    }

                    first_flex_assigned = true;
                    available = 0;
                } else {
                    // Non-flex or subsequent flex: render at natural width
                    let max_w = width.saturating_sub(used_width);
                    if *id == TokenId::StatusIcon {
                        for (text, style) in &ctx.status_icon_spans {
                            spans.push(Span::styled(text.clone(), *style));
                            used_width += display_width(text);
                        }
                    } else if *id == TokenId::GitStats {
                        let (git_spans, git_width) = ctx.git_stats_spans(max_w);
                        for (text, style) in git_spans {
                            spans.push(Span::styled(text, style));
                        }
                        used_width += git_width;
                    } else {
                        let text = ctx.resolve(*id);
                        let rendered = if info.natural_width > max_w && max_w > 0 {
                            truncate_with_ellipsis(&text, max_w)
                        } else {
                            text
                        };
                        spans.push(Span::styled(rendered, ctx.intrinsic_style(*id)));
                        used_width += info.natural_width.min(max_w);
                    }
                }
            }
        }
    }

    // Slack between left and right segments (where {fill} sat).
    if slack > 0 {
        spans.push(Span::raw(" ".repeat(slack)));
        used_width += slack;
    }

    // Render right segment
    for info in right {
        match &info.token {
            Token::Literal(s) => {
                spans.push(Span::raw(s.clone()));
                used_width += info.natural_width;
            }
            Token::Fill => {}
            Token::Field(id) => {
                let max_w = width.saturating_sub(used_width);
                if *id == TokenId::StatusIcon {
                    for (text, style) in &ctx.status_icon_spans {
                        spans.push(Span::styled(text.clone(), *style));
                        used_width += display_width(text);
                    }
                } else if *id == TokenId::GitStats {
                    let (git_spans, git_width) = ctx.git_stats_spans(max_w);
                    for (text, style) in git_spans {
                        spans.push(Span::styled(text, style));
                    }
                    used_width += git_width;
                } else {
                    let text = ctx.resolve(*id);
                    let rendered = if info.natural_width > max_w && max_w > 0 {
                        truncate_with_ellipsis(&text, max_w)
                    } else {
                        text
                    };
                    spans.push(Span::styled(rendered, ctx.intrinsic_style(*id)));
                    used_width += info.natural_width.min(max_w);
                }
            }
        }
    }

    // Fill any remaining width with spaces so the line reaches `width`
    // (important for background coloring in selected rows)
    if used_width < width {
        spans.push(Span::raw(" ".repeat(width - used_width)));
    }

    spans
}

fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(1))
        .sum()
}

fn truncate_with_ellipsis(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if display_width(s) <= max_width {
        return s.to_string();
    }
    if max_width == 1 {
        return "\u{2026}".to_string();
    }

    let mut out = String::new();
    let mut width = 0;
    for c in s.chars() {
        let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
        if width + char_width + 1 > max_width {
            break;
        }
        out.push(c);
        width += char_width;
    }
    let trimmed = out.trim_end();
    let mut result = trimmed.to_string();
    result.push('\u{2026}');
    result
}

#[cfg(test)]
mod tests {
    use super::super::context::RowContext;
    use super::super::parser::{Token, TokenId};
    use super::*;
    use crate::multiplexer::AgentPane;
    use crate::ui::theme::ThemePalette;
    use std::path::PathBuf;

    fn test_palette() -> &'static ThemePalette {
        use crate::config::{ThemeMode, ThemeScheme};
        Box::leak(Box::new(ThemePalette::for_scheme(
            ThemeScheme::Default,
            ThemeMode::Dark,
        )))
    }

    fn test_agent(name: &str) -> AgentPane {
        AgentPane {
            session: "session".to_string(),
            window_name: format!("wm-{}", name),
            pane_id: "%1".to_string(),
            window_id: "@1".to_string(),
            path: PathBuf::from(format!("/tmp/{}", name)),
            pane_title: None,
            status: None,
            status_ts: None,
            updated_ts: None,
            window_cmd: None,
            agent_command: None,
        }
    }

    fn make_context(agent: &AgentPane) -> RowContext {
        // Build a minimal RowContext manually for unit tests
        RowContext {
            agent,
            primary: "feature-auth".to_string(),
            secondary: "myproject".to_string(),
            pane_suffix: String::new(),
            elapsed: "5:23".to_string(),
            status_icon_spans: vec![("💤".to_string(), ratatui::style::Style::default())],
            status_color: ratatui::style::Color::Reset,
            pane_title: None,
            git_status: None,
            is_stale: false,
            is_active: false,
            is_selected: false,
            palette: test_palette(),
            agent_icon: String::new(),
            agent_label: String::new(),
        }
    }

    #[test]
    fn render_line_with_fill() {
        let agent = test_agent("foo");
        let ctx = make_context(&agent);
        let tokens = vec![
            Token::Field(TokenId::Primary),
            Token::Literal(" ".to_string()),
            Token::Fill,
            Token::Literal(" ".to_string()),
            Token::Field(TokenId::Elapsed),
        ];
        let spans = render_line(&ctx, &tokens, 20);
        let text: String = spans.iter().map(|s| s.content.clone()).collect();
        // primary = "feature-auth" (12 cols), elapsed = "5:23" (4 cols), 2 spaces, fill = 2
        // left gets 20 - 4 - 2 = 14; primary is 12 so padded by 2
        assert!(text.contains("feature-auth"));
        assert!(text.contains("5:23"));
    }

    #[test]
    fn render_line_narrow_truncates_flex() {
        let agent = test_agent("foo");
        let ctx = make_context(&agent);
        let tokens = vec![
            Token::Field(TokenId::Primary),
            Token::Fill,
            Token::Field(TokenId::Elapsed),
        ];
        let spans = render_line(&ctx, &tokens, 10);
        let text: String = spans.iter().map(|s| s.content.clone()).collect();
        // elapsed = 4, available = 10 - 4 = 6, primary truncated to ~5 + ellipsis
        assert!(text.contains("5:23"));
        assert!(text.contains('…'));
    }

    #[test]
    fn render_line_drops_right_token_when_narrow() {
        let agent = test_agent("foo");
        let ctx = make_context(&agent);
        let tokens = vec![
            Token::Field(TokenId::Primary),
            Token::Literal(" ".to_string()),
            Token::Fill,
            Token::Literal(" ".to_string()),
            Token::Field(TokenId::Elapsed),
        ];
        // Width of 4: right (5) + left fixed (1) > 4, so elapsed is dropped,
        // then primary is truncated to fit.
        let spans = render_line(&ctx, &tokens, 4);
        let text: String = spans.iter().map(|s| s.content.clone()).collect();
        // Elapsed should be dropped, primary truncated
        assert!(!text.contains("5:23"));
        assert!(text.contains('…'));
    }

    #[test]
    fn empty_line_when_all_variable_tokens_empty() {
        let agent = test_agent("foo");
        let ctx = RowContext {
            agent: &agent,
            primary: String::new(),
            secondary: String::new(),
            pane_suffix: String::new(),
            elapsed: String::new(),
            status_icon_spans: vec![],
            status_color: ratatui::style::Color::Reset,
            pane_title: None,
            git_status: None,
            is_stale: false,
            is_active: false,
            is_selected: false,
            palette: test_palette(),
            agent_icon: String::new(),
            agent_label: String::new(),
        };
        let tokens = vec![
            Token::Field(TokenId::PaneTitle),
            Token::Literal(" ".to_string()),
            Token::Fill,
        ];
        assert!(is_empty_line(&tokens, &ctx));
    }

    #[test]
    fn non_empty_line_with_literal_content() {
        let agent = test_agent("foo");
        let ctx = make_context(&agent);
        let tokens = vec![
            Token::Literal("▌ ".to_string()),
            Token::Field(TokenId::Primary),
        ];
        assert!(!is_empty_line(&tokens, &ctx));
    }

    #[test]
    fn empty_line_with_only_whitespace_literal() {
        let agent = test_agent("foo");
        let ctx = RowContext {
            agent: &agent,
            primary: String::new(),
            secondary: String::new(),
            pane_suffix: String::new(),
            elapsed: String::new(),
            status_icon_spans: vec![],
            status_color: ratatui::style::Color::Reset,
            pane_title: None,
            git_status: None,
            is_stale: false,
            is_active: false,
            is_selected: false,
            palette: test_palette(),
            agent_icon: String::new(),
            agent_label: String::new(),
        };
        let tokens = vec![
            Token::Literal("   ".to_string()),
            Token::Field(TokenId::PaneTitle),
        ];
        assert!(is_empty_line(&tokens, &ctx));
    }
}
