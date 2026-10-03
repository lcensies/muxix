//! Layout DSL node model.
//!
//! A [`Node`] tree is a declarative description of a screen: nested row/column
//! containers, interpolated text, data-bound lists, conditionals, and
//! "native" leaves that host hand-written widgets the DSL places but does not
//! draw. Trees are authored in YAML (see [`super::parse`]) and rendered over
//! ratatui (see [`super::render`]).

use ratatui::layout::Constraint;
use ratatui::style::{Color, Modifier, Style};
use serde::Deserialize;

use crate::ui::theme::ThemePalette;

/// How a node is sized within its parent container, mapped to a ratatui
/// [`Constraint`]. Authored as a single-key map, e.g. `{ fill: 1 }`,
/// `{ length: 3 }`, `{ min: 10 }`, `{ percentage: 40 }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Size {
    Length(u16),
    Min(u16),
    Max(u16),
    Percentage(u16),
    Fill(u16),
}

impl Default for Size {
    fn default() -> Self {
        Size::Fill(1)
    }
}

impl Size {
    pub fn constraint(self) -> Constraint {
        match self {
            Size::Length(n) => Constraint::Length(n),
            Size::Min(n) => Constraint::Min(n),
            Size::Max(n) => Constraint::Max(n),
            Size::Percentage(n) => Constraint::Percentage(n),
            Size::Fill(n) => Constraint::Fill(n),
        }
    }

    /// Fixed row height when laying items vertically; non-`Length` sizes are
    /// treated as a single line.
    pub fn fixed_height(self) -> u16 {
        match self {
            Size::Length(n) => n.max(1),
            _ => 1,
        }
    }
}

/// A foreground/background/modifier style, with colors resolvable against the
/// theme palette by name (`accent`, `dimmed`, ...) or any literal ratatui color
/// (`#a6e3a1`, `red`, `colour42`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct StyleSpec {
    #[serde(default)]
    pub fg: Option<String>,
    #[serde(default)]
    pub bg: Option<String>,
    #[serde(default)]
    pub mods: Vec<String>,
}

impl StyleSpec {
    pub fn resolve(&self, theme: &ThemePalette) -> Style {
        let mut style = Style::default();
        if let Some(fg) = self.fg.as_deref().and_then(|c| resolve_color(c, theme)) {
            style = style.fg(fg);
        }
        if let Some(bg) = self.bg.as_deref().and_then(|c| resolve_color(c, theme)) {
            style = style.bg(bg);
        }
        for m in &self.mods {
            if let Some(modifier) = resolve_modifier(m) {
                style = style.add_modifier(modifier);
            }
        }
        style
    }
}

/// Resolve a color name: theme palette names first, then literal ratatui colors.
pub fn resolve_color(name: &str, theme: &ThemePalette) -> Option<Color> {
    let themed = match name {
        "current_row_bg" => theme.current_row_bg,
        "highlight_row_bg" => theme.highlight_row_bg,
        "current_worktree_fg" => theme.current_worktree_fg,
        "dimmed" => theme.dimmed,
        "text" => theme.text,
        "border" => theme.border,
        "help_border" => theme.help_border,
        "help_muted" => theme.help_muted,
        "header" => theme.header,
        "keycap" => theme.keycap,
        "info" => theme.info,
        "success" => theme.success,
        "warning" => theme.warning,
        "danger" => theme.danger,
        "accent" => theme.accent,
        other => return other.parse::<Color>().ok(),
    };
    Some(themed)
}

fn resolve_modifier(name: &str) -> Option<Modifier> {
    match name.to_ascii_lowercase().as_str() {
        "bold" => Some(Modifier::BOLD),
        "dim" => Some(Modifier::DIM),
        "italic" => Some(Modifier::ITALIC),
        "underline" | "underlined" => Some(Modifier::UNDERLINED),
        "reversed" | "reverse" => Some(Modifier::REVERSED),
        "crossed_out" | "strikethrough" => Some(Modifier::CROSSED_OUT),
        _ => None,
    }
}

/// A node in the layout tree. Serialized with an internal `type` tag.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Node {
    /// Horizontal container: children laid left-to-right.
    Row {
        #[serde(default)]
        size: Size,
        #[serde(default)]
        gap: u16,
        #[serde(default)]
        children: Vec<Node>,
    },
    /// Vertical container: children laid top-to-bottom.
    Col {
        #[serde(default)]
        size: Size,
        #[serde(default)]
        gap: u16,
        #[serde(default)]
        children: Vec<Node>,
    },
    /// Interpolated, styled text line. `content` may contain `{field}`
    /// placeholders (resolved from the list-item scope, then the global
    /// context) and inline `#[fg=...]` tmux-style directives.
    Text {
        #[serde(default = "Size::default_text")]
        size: Size,
        content: String,
        #[serde(default)]
        style: Option<StyleSpec>,
    },
    /// A vertical, data-bound list. Rows come from `ctx.list(source)`; each is
    /// rendered with `item`, except section-header rows which use `header`
    /// (falling back to a bold `{header}` line). The selected row is
    /// highlighted and kept in view.
    List {
        #[serde(default)]
        size: Size,
        source: String,
        item: Box<Node>,
        #[serde(default)]
        header: Option<Box<Node>>,
        #[serde(default)]
        empty: Option<Box<Node>>,
    },
    /// Blank filler.
    Spacer {
        #[serde(default)]
        size: Size,
    },
    /// A hosted native widget, drawn by `ctx.render_native(name, ...)`.
    Native {
        #[serde(default)]
        size: Size,
        name: String,
    },
    /// Conditional: render `then` when `ctx.flag(cond)` is true, else `else`.
    If {
        #[serde(default)]
        size: Size,
        cond: String,
        then: Box<Node>,
        #[serde(default, rename = "else")]
        otherwise: Option<Box<Node>>,
    },
}

impl Size {
    /// Text defaults to a single line rather than filling.
    fn default_text() -> Size {
        Size::Length(1)
    }
}

impl Node {
    /// The constraint this node contributes to its parent's layout.
    pub fn size(&self) -> Size {
        match self {
            Node::Row { size, .. }
            | Node::Col { size, .. }
            | Node::Text { size, .. }
            | Node::List { size, .. }
            | Node::Spacer { size, .. }
            | Node::Native { size, .. }
            | Node::If { size, .. } => *size,
        }
    }
}
