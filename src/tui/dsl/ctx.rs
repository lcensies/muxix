//! The render context: the data side of the layout DSL.
//!
//! A [`RenderCtx`] projects application state into the values the DSL binds to —
//! scalar/styled fields, boolean flags, list rows — and hosts native widgets.
//! Each TUI implements this trait over its own app state.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::ui::theme::ThemePalette;

/// A resolved field value: either plain text (styled by the node) or
/// pre-styled spans (for multi-colored content like status icons).
#[derive(Debug, Clone)]
pub enum FieldValue {
    Text(String),
    Spans(Vec<(String, Style)>),
}

impl FieldValue {
    pub fn text(s: impl Into<String>) -> Self {
        FieldValue::Text(s.into())
    }
}

/// One row of a [`Node::List`](super::node::Node::List).
///
/// A normal row carries `fields` that the item template interpolates. A row
/// with `header: Some(_)` is a section heading rendered with the list's header
/// template instead.
#[derive(Debug, Clone, Default)]
pub struct ListItem {
    pub fields: Vec<(String, FieldValue)>,
    pub selected: bool,
    pub header: Option<String>,
}

impl ListItem {
    pub fn header(title: impl Into<String>) -> Self {
        ListItem {
            fields: Vec::new(),
            selected: false,
            header: Some(title.into()),
        }
    }

    pub fn row(fields: Vec<(String, FieldValue)>, selected: bool) -> Self {
        ListItem {
            fields,
            selected,
            header: None,
        }
    }

    pub fn get(&self, name: &str) -> Option<&FieldValue> {
        self.fields.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }
}

/// The data + hosting surface a [`Node`](super::node::Node) tree renders against.
pub trait RenderCtx {
    /// Active theme palette for color/name resolution.
    fn theme(&self) -> &ThemePalette;

    /// Resolve a global (non-list) field for text interpolation.
    fn field(&self, _name: &str) -> Option<FieldValue> {
        None
    }

    /// Evaluate a boolean flag for `Node::If`.
    fn flag(&self, _name: &str) -> bool {
        false
    }

    /// Produce the rows for a `Node::List` source.
    fn list(&self, _source: &str) -> Vec<ListItem> {
        Vec::new()
    }

    /// Draw a hosted native widget into `area`.
    fn render_native(&mut self, _name: &str, _area: Rect, _frame: &mut Frame) {}
}
