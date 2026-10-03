//! Text interpolation: turn a template string into styled ratatui spans.
//!
//! Grammar (intentionally small):
//! - `{name}`      — a field, resolved from the list-item scope then the global
//!   context. Text fields may themselves carry `#[...]` directives; span fields
//!   are inserted with their own styles.
//! - `{{` / `}}`   — literal braces.
//! - `#[fg=...]`   — inline tmux-style directives in literal text (see
//!   [`crate::tmux_style`]).

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::tmux_style::parse_tmux_styles;
use crate::ui::theme::ThemePalette;

use super::ctx::{FieldValue, ListItem, RenderCtx};
use super::node::StyleSpec;

/// Render `content` into a styled [`Line`], resolving fields from `scope` (a
/// list item, if any) then the global `ctx`.
pub fn render_line(
    content: &str,
    style: Option<&StyleSpec>,
    theme: &ThemePalette,
    scope: Option<&ListItem>,
    ctx: &dyn RenderCtx,
) -> Line<'static> {
    let base = style.map(|s| s.resolve(theme)).unwrap_or_default();
    let mut spans: Vec<Span<'static>> = Vec::new();

    let mut literal = String::new();
    let mut chars = content.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                literal.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                literal.push('}');
            }
            '{' => {
                flush_literal(&mut literal, base, &mut spans);
                let mut name = String::new();
                for nc in chars.by_ref() {
                    if nc == '}' {
                        break;
                    }
                    name.push(nc);
                }
                push_field(&name, base, theme, scope, ctx, &mut spans);
            }
            other => literal.push(other),
        }
    }
    flush_literal(&mut literal, base, &mut spans);

    if spans.is_empty() {
        spans.push(Span::styled(String::new(), base));
    }
    Line::from(spans)
}

fn flush_literal(literal: &mut String, base: Style, out: &mut Vec<Span<'static>>) {
    if literal.is_empty() {
        return;
    }
    for (text, style) in parse_tmux_styles(literal, base) {
        out.push(Span::styled(text, style));
    }
    literal.clear();
}

fn push_field(
    name: &str,
    base: Style,
    theme: &ThemePalette,
    scope: Option<&ListItem>,
    ctx: &dyn RenderCtx,
    out: &mut Vec<Span<'static>>,
) {
    let value = scope
        .and_then(|s| s.get(name).cloned())
        .or_else(|| ctx.field(name));
    let Some(value) = value else { return };
    match value {
        FieldValue::Text(s) => {
            for (text, style) in parse_tmux_styles(&s, base) {
                out.push(Span::styled(text, style));
            }
        }
        FieldValue::Spans(spans) => {
            let _ = theme;
            for (text, style) in spans {
                out.push(Span::styled(text, style));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ThemeMode};

    struct Ctx {
        theme: ThemePalette,
    }
    impl RenderCtx for Ctx {
        fn theme(&self) -> &ThemePalette {
            &self.theme
        }
        fn field(&self, name: &str) -> Option<FieldValue> {
            match name {
                "title" => Some(FieldValue::text("Dashboard")),
                _ => None,
            }
        }
    }

    fn ctx() -> Ctx {
        Ctx {
            theme: ThemePalette::from_config(&Config::default().theme, ThemeMode::Dark),
        }
    }

    fn flat(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn interpolates_global_field() {
        let c = ctx();
        let line = render_line("App: {title}", None, c.theme(), None, &c);
        assert_eq!(flat(&line), "App: Dashboard");
    }

    #[test]
    fn item_scope_overrides_global() {
        let c = ctx();
        let item = ListItem::row(vec![("title".into(), FieldValue::text("Agent"))], false);
        let line = render_line("{title}", None, c.theme(), Some(&item), &c);
        assert_eq!(flat(&line), "Agent");
    }

    #[test]
    fn literal_braces_escape() {
        let c = ctx();
        let line = render_line("{{x}}", None, c.theme(), None, &c);
        assert_eq!(flat(&line), "{x}");
    }

    #[test]
    fn missing_field_is_empty() {
        let c = ctx();
        let line = render_line("[{nope}]", None, c.theme(), None, &c);
        assert_eq!(flat(&line), "[]");
    }

    #[test]
    fn span_field_keeps_segments() {
        let c = ctx();
        let item = ListItem::row(
            vec![(
                "icon".into(),
                FieldValue::Spans(vec![
                    ("● ".into(), Style::default()),
                    ("ok".into(), Style::default()),
                ]),
            )],
            false,
        );
        let line = render_line("{icon}", None, c.theme(), Some(&item), &c);
        assert_eq!(flat(&line), "● ok");
    }
}
