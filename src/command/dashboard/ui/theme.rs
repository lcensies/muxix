//! Theme palette for dashboard colors.

use ratatui::style::Color;

use crate::config::Theme;

/// All customizable colors used in the dashboard UI.
/// Constructed from a [Theme] variant.
pub struct ThemePalette {
    /// Background for the current worktree row
    pub current_row_bg: Color,
    /// Background for the selected/highlighted row
    pub highlight_row_bg: Color,
    /// Text color for the current worktree name
    pub current_worktree_fg: Color,
    /// Dimmed/secondary text (borders, stale agents, spinners, inactive items)
    pub dimmed: Color,
    /// Primary text color (worktree names, descriptions, help text)
    pub text: Color,
    /// Help overlay border color
    pub help_border: Color,
    /// Help overlay separator/bottom text color
    pub help_muted: Color,
}

impl ThemePalette {
    pub fn from_theme(theme: Theme) -> Self {
        match theme {
            Theme::Dark => Self::dark(),
            Theme::Light => Self::light(),
        }
    }

    fn dark() -> Self {
        Self {
            current_row_bg: Color::Rgb(35, 40, 35),
            highlight_row_bg: Color::Rgb(50, 50, 55),
            current_worktree_fg: Color::White,
            dimmed: Color::DarkGray,
            text: Color::White,
            help_border: Color::Rgb(100, 100, 120),
            help_muted: Color::Rgb(70, 70, 80),
        }
    }

    fn light() -> Self {
        Self {
            current_row_bg: Color::Rgb(215, 230, 215),
            highlight_row_bg: Color::Rgb(200, 200, 210),
            current_worktree_fg: Color::Black,
            dimmed: Color::Gray,
            text: Color::Black,
            help_border: Color::Rgb(160, 160, 175),
            help_muted: Color::Rgb(130, 130, 145),
        }
    }
}
