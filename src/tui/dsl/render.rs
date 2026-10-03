//! Renderer: draw a [`Node`] tree over ratatui.
//!
//! Composition (containers, constraints, lists, conditionals) is owned by the
//! DSL; specialized interactive widgets are placed via `Node::Native` and drawn
//! by the host through [`RenderCtx::render_native`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Text;
use ratatui::widgets::Paragraph;

use crate::ui::theme::ThemePalette;

use super::ctx::{ListItem, RenderCtx};
use super::node::Node;
use super::text::render_line;

/// Render a node tree into `area`.
pub fn render(frame: &mut Frame, area: Rect, root: &Node, ctx: &mut dyn RenderCtx) {
    let theme = *ctx.theme();
    render_node(frame, area, root, &theme, ctx, None);
}

fn render_node(
    frame: &mut Frame,
    area: Rect,
    node: &Node,
    theme: &ThemePalette,
    ctx: &mut dyn RenderCtx,
    scope: Option<&ListItem>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match node {
        Node::Row { children, gap, .. } => render_container(
            frame,
            area,
            children,
            *gap,
            Direction::Horizontal,
            theme,
            ctx,
            scope,
        ),
        Node::Col { children, gap, .. } => render_container(
            frame,
            area,
            children,
            *gap,
            Direction::Vertical,
            theme,
            ctx,
            scope,
        ),
        Node::Text { content, style, .. } => {
            let line = render_line(content, style.as_ref(), theme, scope, ctx);
            frame.render_widget(Paragraph::new(line), area);
        }
        Node::Spacer { .. } => {}
        Node::Native { name, .. } => ctx.render_native(name, area, frame),
        Node::If {
            cond,
            then,
            otherwise,
            ..
        } => {
            if ctx.flag(cond) {
                render_node(frame, area, then, theme, ctx, scope);
            } else if let Some(other) = otherwise {
                render_node(frame, area, other, theme, ctx, scope);
            }
        }
        Node::List {
            source,
            item,
            header,
            empty,
            ..
        } => render_list(
            frame,
            area,
            source,
            item,
            header.as_deref(),
            empty.as_deref(),
            theme,
            ctx,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn render_container(
    frame: &mut Frame,
    area: Rect,
    children: &[Node],
    gap: u16,
    dir: Direction,
    theme: &ThemePalette,
    ctx: &mut dyn RenderCtx,
    scope: Option<&ListItem>,
) {
    if children.is_empty() {
        return;
    }
    let constraints: Vec<Constraint> = children.iter().map(|c| c.size().constraint()).collect();
    let chunks = Layout::default()
        .direction(dir)
        .spacing(gap)
        .constraints(constraints)
        .split(area);
    for (child, chunk) in children.iter().zip(chunks.iter()) {
        render_node(frame, *chunk, child, theme, ctx, scope);
    }
}

#[allow(clippy::too_many_arguments)]
fn render_list(
    frame: &mut Frame,
    area: Rect,
    source: &str,
    item_tpl: &Node,
    header_tpl: Option<&Node>,
    empty_tpl: Option<&Node>,
    theme: &ThemePalette,
    ctx: &mut dyn RenderCtx,
) {
    let items = ctx.list(source);

    if items.is_empty() {
        if let Some(empty) = empty_tpl {
            render_node(frame, area, empty, theme, ctx, None);
        }
        return;
    }

    let heights: Vec<u16> = items
        .iter()
        .map(|it| {
            if it.header.is_some() {
                header_tpl.map(|n| n.size().fixed_height()).unwrap_or(1)
            } else {
                item_tpl.size().fixed_height()
            }
        })
        .collect();

    let selected = items.iter().position(|it| it.selected);
    let start = scroll_start(&heights, selected, area.height);

    let mut y = area.y;
    let bottom = area.y + area.height;
    for (idx, it) in items.iter().enumerate().skip(start) {
        let h = heights[idx];
        if y >= bottom {
            break;
        }
        let visible_h = h.min(bottom - y);
        let row = Rect {
            x: area.x,
            y,
            width: area.width,
            height: visible_h,
        };

        if it.selected {
            frame
                .buffer_mut()
                .set_style(row, Style::default().bg(theme.highlight_row_bg));
        }

        if let Some(title) = &it.header {
            // The header title is exposed to the template as `{header}`.
            let hscope = ListItem::row(
                vec![("header".into(), super::ctx::FieldValue::text(title.clone()))],
                false,
            );
            match header_tpl {
                Some(tpl) => render_node(frame, row, tpl, theme, ctx, Some(&hscope)),
                None => {
                    let line = render_line(
                        "{header}",
                        Some(&super::node::StyleSpec {
                            fg: Some("header".into()),
                            bg: None,
                            mods: vec!["bold".into()],
                        }),
                        theme,
                        Some(&hscope),
                        ctx,
                    );
                    frame.render_widget(Paragraph::new(Text::from(line)), row);
                }
            }
        } else {
            render_node(frame, row, item_tpl, theme, ctx, Some(it));
        }

        y += h;
    }
}

/// Smallest start index that keeps the selected row visible (bottom-anchored
/// when scrolling), clamped so we never scroll past the end unnecessarily.
fn scroll_start(heights: &[u16], selected: Option<usize>, viewport: u16) -> usize {
    let Some(selected) = selected else { return 0 };
    let mut start = 0usize;
    loop {
        let used: u16 = heights[start..=selected].iter().sum();
        if used <= viewport || start == selected {
            return start;
        }
        start += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ThemeMode};
    use crate::tui::dsl::ctx::FieldValue;
    use crate::tui::dsl::parse::parse_yaml;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    struct Ctx {
        theme: ThemePalette,
        agents: Vec<ListItem>,
        wide: bool,
    }
    impl RenderCtx for Ctx {
        fn theme(&self) -> &ThemePalette {
            &self.theme
        }
        fn field(&self, name: &str) -> Option<FieldValue> {
            match name {
                "title" => Some(FieldValue::text("Agents")),
                _ => None,
            }
        }
        fn flag(&self, name: &str) -> bool {
            name == "wide" && self.wide
        }
        fn list(&self, source: &str) -> Vec<ListItem> {
            if source == "agents" {
                self.agents.clone()
            } else {
                Vec::new()
            }
        }
    }

    fn theme() -> ThemePalette {
        ThemePalette::from_config(&Config::default().theme, ThemeMode::Dark)
    }

    fn buffer_text(t: &Terminal<TestBackend>) -> String {
        t.backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn renders_header_and_list_rows() {
        let spec = r##"
type: col
children:
  - type: text
    size: { length: 1 }
    content: "== {title} =="
    style: { fg: header, mods: [bold] }
  - type: list
    size: { fill: 1 }
    source: agents
    header:
      type: text
      content: "# {header}"
    item:
      type: row
      children:
        - { type: text, size: { length: 4 }, content: "{icon}" }
        - { type: text, size: { fill: 1 }, content: "{name}" }
"##;
        let root = parse_yaml(spec).expect("parse");
        let mut ctx = Ctx {
            theme: theme(),
            agents: vec![
                ListItem::header("Needs input"),
                ListItem::row(
                    vec![
                        ("icon".into(), FieldValue::text("[!]")),
                        ("name".into(), FieldValue::text("auth")),
                    ],
                    true,
                ),
                ListItem::header("Working"),
                ListItem::row(
                    vec![
                        ("icon".into(), FieldValue::text("[~]")),
                        ("name".into(), FieldValue::text("api")),
                    ],
                    false,
                ),
            ],
            wide: false,
        };
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &root, &mut ctx))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("== Agents =="), "title missing: {text:?}");
        assert!(text.contains("# Needs input"), "header missing");
        assert!(text.contains("auth"), "selected row missing");
        assert!(text.contains("# Working"), "second header missing");
        assert!(text.contains("api"), "second row missing");
    }

    #[test]
    fn conditional_selects_branch() {
        let spec = r#"
type: if
cond: wide
then: { type: text, content: "WIDE" }
else: { type: text, content: "narrow" }
"#;
        let root = parse_yaml(spec).unwrap();
        for (wide, expect) in [(true, "WIDE"), (false, "narrow")] {
            let mut ctx = Ctx {
                theme: theme(),
                agents: vec![],
                wide,
            };
            let backend = TestBackend::new(20, 3);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|f| render(f, f.area(), &root, &mut ctx))
                .unwrap();
            assert!(buffer_text(&terminal).contains(expect));
        }
    }

    #[test]
    fn empty_list_uses_empty_template() {
        let spec = r#"
type: list
source: agents
empty: { type: text, content: "no agents" }
item: { type: text, content: "{name}" }
"#;
        let root = parse_yaml(spec).unwrap();
        let mut ctx = Ctx {
            theme: theme(),
            agents: vec![],
            wide: false,
        };
        let backend = TestBackend::new(20, 3);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &root, &mut ctx))
            .unwrap();
        assert!(buffer_text(&terminal).contains("no agents"));
    }
}
