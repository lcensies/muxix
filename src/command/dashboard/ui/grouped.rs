//! Status-grouped ("sorted by status") agents view, rendered through the
//! layout DSL ([`crate::tui::dsl`]).
//!
//! The layout is described by [`DEFAULT_AGENTS_SPEC`] (or an on-disk override at
//! `.workmux/ui/agents.yaml` for live prototyping). Agent rows and section
//! headers are projected from [`App`] via [`AgentsCtx`].

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::tui::dsl::{self, ListItem, RenderCtx};
use crate::ui::theme::ThemePalette;

use super::super::app::App;

/// Built-in layout for the grouped agents view. Overridable at
/// `.workmux/ui/agents.yaml`. Must always parse.
pub const DEFAULT_AGENTS_SPEC: &str = r#"
type: list
source: agents
header:
  type: text
  content: "  {header}"
  style: { fg: accent, mods: [bold] }
item:
  type: row
  gap: 1
  children:
    - { type: text, size: { length: 12 }, content: "  {status}" }
    - { type: text, size: { length: 16 }, content: "{project}", style: { fg: dimmed } }
    - { type: text, size: { fill: 2 }, content: "{name}" }
    - { type: text, size: { length: 7 }, content: "{time}", style: { fg: dimmed } }
    - { type: text, size: { fill: 3 }, content: "{title}", style: { fg: dimmed } }
"#;

/// Render context projecting the dashboard's agents into the DSL.
struct AgentsCtx<'a> {
    theme: &'a ThemePalette,
    items: Vec<ListItem>,
}

impl RenderCtx for AgentsCtx<'_> {
    fn theme(&self) -> &ThemePalette {
        self.theme
    }

    fn list(&self, source: &str) -> Vec<ListItem> {
        if source == "agents" {
            self.items.clone()
        } else {
            Vec::new()
        }
    }
}

/// Render the grouped agents view into `area`.
pub fn render_grouped_agents(f: &mut Frame, app: &mut App, area: Rect) {
    // Pick up live edits to the override spec.
    app.agents_spec.reload_if_changed();

    // Reserve a banner line when the override currently fails to parse, so a
    // typo while prototyping is visible rather than silently ignored.
    let body = if let Some(err) = app.agents_spec.error() {
        let chunks =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).split(area);
        let line = Line::from(Span::styled(
            format!(" spec error: {err}"),
            Style::default().fg(app.palette.danger),
        ));
        f.render_widget(Paragraph::new(line), chunks[0]);
        chunks[1]
    } else {
        area
    };

    if app.agents.is_empty() {
        let line = Line::from(Span::styled(
            " No active agents.",
            Style::default().fg(app.palette.dimmed),
        ));
        f.render_widget(Paragraph::new(line), body);
        return;
    }

    let items = app.grouped_agent_items();
    let theme = app.palette;
    let node = app.agents_spec.node();
    let mut ctx = AgentsCtx {
        theme: &theme,
        items,
    };
    dsl::render(f, body, node, &mut ctx);
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_AGENTS_SPEC;

    #[test]
    fn embedded_spec_parses() {
        // App::new builds a SpecSource from this and panics on a bad spec;
        // catch typos here instead.
        crate::tui::dsl::parse::parse_yaml(DEFAULT_AGENTS_SPEC)
            .expect("default agents spec must parse");
    }
}
