//! Theme palette for dashboard colors.

use ratatui::style::Color;

use crate::config::Theme;

/// Number of available dark theme variants (default + named schemes).
pub const DARK_SCHEME_COUNT: u8 = 11;

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

    /// Get a dark theme variant by index (0 = default, 1-10 = named schemes).
    pub fn dark_variant(index: u8) -> Self {
        match index {
            1 => Self::emberforge(),
            2 => Self::glacier_signal(),
            3 => Self::obsidian_pop(),
            4 => Self::slate_garden(),
            5 => Self::phosphor_arcade(),
            6 => Self::lasergrid(),
            7 => Self::mossfire(),
            8 => Self::night_sorbet(),
            9 => Self::graphite_code(),
            10 => Self::festival_circuit(),
            _ => Self::dark(),
        }
    }

    /// Name of a dark theme variant by index.
    pub fn dark_variant_name(index: u8) -> &'static str {
        match index {
            1 => "Emberforge",
            2 => "Glacier Signal",
            3 => "Obsidian Pop",
            4 => "Slate Garden",
            5 => "Phosphor Arcade",
            6 => "Lasergrid",
            7 => "Mossfire",
            8 => "Night Sorbet",
            9 => "Graphite Code",
            10 => "Festival Circuit",
            _ => "Default",
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

    // ── Named dark schemes ──────────────────────────────────────

    /// Copper-and-amber glow with cool cyan state contrast.
    fn emberforge() -> Self {
        Self {
            current_row_bg: Color::Rgb(32, 22, 18),
            highlight_row_bg: Color::Rgb(50, 34, 27),
            current_worktree_fg: Color::Rgb(255, 245, 230),
            dimmed: Color::Rgb(150, 124, 108),
            text: Color::Rgb(231, 214, 198),
            border: Color::Rgb(104, 78, 63),
            help_border: Color::Rgb(146, 105, 82),
            help_muted: Color::Rgb(175, 145, 127),

            header: Color::Rgb(255, 176, 92),
            keycap: Color::Rgb(255, 221, 138),
            info: Color::Rgb(103, 196, 207),
            success: Color::Rgb(145, 211, 118),
            warning: Color::Rgb(255, 189, 92),
            danger: Color::Rgb(228, 108, 92),
            accent: Color::Rgb(220, 136, 108),
        }
    }

    /// Icy blues with clean, modern contrast and restrained warmth.
    fn glacier_signal() -> Self {
        Self {
            current_row_bg: Color::Rgb(16, 27, 41),
            highlight_row_bg: Color::Rgb(25, 41, 60),
            current_worktree_fg: Color::Rgb(237, 247, 255),
            dimmed: Color::Rgb(111, 129, 149),
            text: Color::Rgb(198, 216, 232),
            border: Color::Rgb(64, 88, 112),
            help_border: Color::Rgb(88, 119, 150),
            help_muted: Color::Rgb(123, 143, 163),

            header: Color::Rgb(118, 187, 255),
            keycap: Color::Rgb(166, 232, 255),
            info: Color::Rgb(104, 220, 233),
            success: Color::Rgb(128, 214, 184),
            warning: Color::Rgb(255, 197, 108),
            danger: Color::Rgb(255, 122, 122),
            accent: Color::Rgb(153, 170, 255),
        }
    }

    /// Near-black base with loud, unmistakable status colors.
    fn obsidian_pop() -> Self {
        Self {
            current_row_bg: Color::Rgb(10, 10, 14),
            highlight_row_bg: Color::Rgb(26, 26, 34),
            current_worktree_fg: Color::Rgb(250, 250, 255),
            dimmed: Color::Rgb(118, 118, 132),
            text: Color::Rgb(226, 226, 240),
            border: Color::Rgb(78, 78, 94),
            help_border: Color::Rgb(0, 229, 255),
            help_muted: Color::Rgb(156, 156, 176),

            header: Color::Rgb(255, 64, 196),
            keycap: Color::Rgb(255, 234, 0),
            info: Color::Rgb(0, 214, 255),
            success: Color::Rgb(94, 255, 0),
            warning: Color::Rgb(255, 179, 0),
            danger: Color::Rgb(255, 75, 75),
            accent: Color::Rgb(166, 97, 255),
        }
    }

    /// Soft, desaturated slate tones for long sessions with low fatigue.
    fn slate_garden() -> Self {
        Self {
            current_row_bg: Color::Rgb(27, 31, 38),
            highlight_row_bg: Color::Rgb(39, 46, 54),
            current_worktree_fg: Color::Rgb(236, 236, 228),
            dimmed: Color::Rgb(126, 129, 138),
            text: Color::Rgb(203, 207, 210),
            border: Color::Rgb(82, 90, 99),
            help_border: Color::Rgb(101, 113, 123),
            help_muted: Color::Rgb(139, 145, 150),

            header: Color::Rgb(138, 167, 190),
            keycap: Color::Rgb(196, 176, 118),
            info: Color::Rgb(120, 170, 168),
            success: Color::Rgb(150, 182, 138),
            warning: Color::Rgb(206, 177, 115),
            danger: Color::Rgb(186, 132, 135),
            accent: Color::Rgb(161, 149, 188),
        }
    }

    /// CRT-green dashboard with amber highlights and old-terminal energy.
    fn phosphor_arcade() -> Self {
        Self {
            current_row_bg: Color::Rgb(12, 23, 18),
            highlight_row_bg: Color::Rgb(20, 37, 29),
            current_worktree_fg: Color::Rgb(205, 255, 190),
            dimmed: Color::Rgb(106, 131, 102),
            text: Color::Rgb(170, 224, 158),
            border: Color::Rgb(56, 92, 67),
            help_border: Color::Rgb(95, 126, 82),
            help_muted: Color::Rgb(125, 150, 116),

            header: Color::Rgb(255, 192, 92),
            keycap: Color::Rgb(255, 228, 138),
            info: Color::Rgb(104, 220, 187),
            success: Color::Rgb(143, 240, 132),
            warning: Color::Rgb(248, 191, 89),
            danger: Color::Rgb(255, 110, 87),
            accent: Color::Rgb(132, 209, 255),
        }
    }

    /// Ultraviolet base with acid brights and aggressive synth-club contrast.
    fn lasergrid() -> Self {
        Self {
            current_row_bg: Color::Rgb(14, 10, 25),
            highlight_row_bg: Color::Rgb(27, 19, 46),
            current_worktree_fg: Color::Rgb(244, 240, 255),
            dimmed: Color::Rgb(127, 113, 153),
            text: Color::Rgb(217, 207, 240),
            border: Color::Rgb(87, 68, 133),
            help_border: Color::Rgb(255, 60, 166),
            help_muted: Color::Rgb(160, 141, 188),

            header: Color::Rgb(46, 243, 255),
            keycap: Color::Rgb(234, 255, 71),
            info: Color::Rgb(0, 216, 255),
            success: Color::Rgb(102, 255, 120),
            warning: Color::Rgb(255, 167, 46),
            danger: Color::Rgb(255, 74, 138),
            accent: Color::Rgb(202, 79, 255),
        }
    }

    /// Forest-and-soil palette with natural warmth and subdued saturation.
    fn mossfire() -> Self {
        Self {
            current_row_bg: Color::Rgb(22, 24, 18),
            highlight_row_bg: Color::Rgb(36, 40, 30),
            current_worktree_fg: Color::Rgb(238, 233, 214),
            dimmed: Color::Rgb(126, 128, 104),
            text: Color::Rgb(208, 202, 183),
            border: Color::Rgb(86, 89, 67),
            help_border: Color::Rgb(116, 123, 82),
            help_muted: Color::Rgb(149, 148, 126),

            header: Color::Rgb(172, 196, 118),
            keycap: Color::Rgb(228, 193, 121),
            info: Color::Rgb(103, 171, 182),
            success: Color::Rgb(126, 186, 108),
            warning: Color::Rgb(218, 169, 88),
            danger: Color::Rgb(191, 111, 88),
            accent: Color::Rgb(166, 131, 173),
        }
    }

    /// Soft candy tones adapted for dark mode without washing out contrast.
    fn night_sorbet() -> Self {
        Self {
            current_row_bg: Color::Rgb(27, 24, 38),
            highlight_row_bg: Color::Rgb(42, 37, 58),
            current_worktree_fg: Color::Rgb(250, 245, 248),
            dimmed: Color::Rgb(145, 137, 157),
            text: Color::Rgb(223, 214, 226),
            border: Color::Rgb(93, 84, 112),
            help_border: Color::Rgb(129, 124, 164),
            help_muted: Color::Rgb(166, 159, 179),

            header: Color::Rgb(151, 210, 255),
            keycap: Color::Rgb(255, 235, 178),
            info: Color::Rgb(153, 236, 229),
            success: Color::Rgb(182, 234, 171),
            warning: Color::Rgb(255, 196, 160),
            danger: Color::Rgb(255, 155, 173),
            accent: Color::Rgb(205, 180, 255),
        }
    }

    /// Single-family graphite palette relying on lightness more than hue.
    fn graphite_code() -> Self {
        Self {
            current_row_bg: Color::Rgb(19, 22, 26),
            highlight_row_bg: Color::Rgb(32, 36, 42),
            current_worktree_fg: Color::Rgb(242, 244, 247),
            dimmed: Color::Rgb(116, 124, 132),
            text: Color::Rgb(205, 210, 216),
            border: Color::Rgb(74, 82, 90),
            help_border: Color::Rgb(102, 111, 120),
            help_muted: Color::Rgb(136, 144, 152),

            header: Color::Rgb(226, 230, 235),
            keycap: Color::Rgb(246, 248, 250),
            info: Color::Rgb(188, 196, 204),
            success: Color::Rgb(214, 220, 226),
            warning: Color::Rgb(171, 179, 187),
            danger: Color::Rgb(146, 153, 161),
            accent: Color::Rgb(232, 236, 240),
        }
    }

    /// Saturated jewel tones with strong separation and a lively dashboard feel.
    fn festival_circuit() -> Self {
        Self {
            current_row_bg: Color::Rgb(19, 18, 44),
            highlight_row_bg: Color::Rgb(33, 31, 70),
            current_worktree_fg: Color::Rgb(248, 244, 255),
            dimmed: Color::Rgb(129, 126, 170),
            text: Color::Rgb(219, 214, 241),
            border: Color::Rgb(74, 86, 143),
            help_border: Color::Rgb(100, 119, 196),
            help_muted: Color::Rgb(151, 149, 190),

            header: Color::Rgb(69, 211, 255),
            keycap: Color::Rgb(255, 219, 70),
            info: Color::Rgb(71, 231, 204),
            success: Color::Rgb(117, 226, 96),
            warning: Color::Rgb(255, 156, 63),
            danger: Color::Rgb(255, 92, 132),
            accent: Color::Rgb(178, 101, 255),
        }
    }
}
