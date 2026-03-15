//! Theme palette for dashboard colors.

use ratatui::style::Color;

use crate::config::Theme;

/// All customizable colors used in the dashboard UI.
/// Constructed from a [Theme] variant.
pub struct ThemePalette {
    // --- Base UI elements ---
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
    /// Standard border color
    pub border: Color,
    /// Help overlay border color
    pub help_border: Color,
    /// Help overlay separator/bottom text color
    pub help_muted: Color,

    // --- Semantic colors ---
    /// Table headers, block titles, overlay titles
    pub header: Color,
    /// Jump keys, footer shortcuts, filter prompt
    pub keycap: Color,
    /// Working/live/interactive state, ahead counts
    pub info: Color,
    /// Additions, open PRs, done status, success checks
    pub success: Color,
    /// Modified files, pending checks, behind counts
    pub warning: Color,
    /// Removals, conflicts, closed PRs, destructive actions
    pub danger: Color,
    /// Patch mode, merged PRs, waiting status, diff icons
    pub accent: Color,
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
            current_row_bg: Color::Rgb(24, 34, 46),
            highlight_row_bg: Color::Rgb(40, 48, 62),
            current_worktree_fg: Color::Rgb(244, 248, 255),
            dimmed: Color::Rgb(108, 112, 134),
            text: Color::Rgb(205, 214, 244),
            border: Color::Rgb(58, 74, 94),
            help_border: Color::Rgb(81, 104, 130),
            help_muted: Color::Rgb(112, 126, 144),

            header: Color::Rgb(137, 180, 250),
            keycap: Color::Rgb(249, 226, 175),
            info: Color::Rgb(120, 225, 213),
            success: Color::Rgb(166, 218, 149),
            warning: Color::Rgb(249, 226, 175),
            danger: Color::Rgb(237, 135, 150),
            accent: Color::Rgb(203, 166, 247),
        }
    }

    fn light() -> Self {
        Self {
            current_row_bg: Color::Rgb(215, 230, 215),
            highlight_row_bg: Color::Rgb(200, 200, 210),
            current_worktree_fg: Color::Rgb(76, 79, 105),
            dimmed: Color::Rgb(140, 143, 161),
            text: Color::Rgb(76, 79, 105),
            border: Color::Rgb(160, 160, 175),
            help_border: Color::Rgb(130, 130, 160),
            help_muted: Color::Rgb(140, 143, 161),

            header: Color::Rgb(30, 102, 245),
            keycap: Color::Rgb(223, 142, 29),
            info: Color::Rgb(23, 146, 153),
            success: Color::Rgb(64, 160, 43),
            warning: Color::Rgb(223, 142, 29),
            danger: Color::Rgb(210, 15, 57),
            accent: Color::Rgb(136, 57, 239),
        }
    }
}
