//! Theme palette for dashboard colors.

use ratatui::style::Color;

use crate::config::{ThemeMode, ThemeScheme};

/// All customizable colors used in the dashboard UI.
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
    /// Build a palette for the given scheme and mode.
    pub fn for_scheme(scheme: ThemeScheme, mode: ThemeMode) -> Self {
        match (scheme, mode) {
            (ThemeScheme::Default, ThemeMode::Dark) => Self::default_dark(),
            (ThemeScheme::Default, ThemeMode::Light) => Self::default_light(),
            (ThemeScheme::Emberforge, ThemeMode::Dark) => Self::emberforge_dark(),
            (ThemeScheme::Emberforge, ThemeMode::Light) => Self::emberforge_light(),
            (ThemeScheme::GlacierSignal, ThemeMode::Dark) => Self::glacier_signal_dark(),
            (ThemeScheme::GlacierSignal, ThemeMode::Light) => Self::glacier_signal_light(),
            (ThemeScheme::ObsidianPop, ThemeMode::Dark) => Self::obsidian_pop_dark(),
            (ThemeScheme::ObsidianPop, ThemeMode::Light) => Self::obsidian_pop_light(),
            (ThemeScheme::SlateGarden, ThemeMode::Dark) => Self::slate_garden_dark(),
            (ThemeScheme::SlateGarden, ThemeMode::Light) => Self::slate_garden_light(),
            (ThemeScheme::PhosphorArcade, ThemeMode::Dark) => Self::phosphor_arcade_dark(),
            (ThemeScheme::PhosphorArcade, ThemeMode::Light) => Self::phosphor_arcade_light(),
            (ThemeScheme::Lasergrid, ThemeMode::Dark) => Self::lasergrid_dark(),
            (ThemeScheme::Lasergrid, ThemeMode::Light) => Self::lasergrid_light(),
            (ThemeScheme::Mossfire, ThemeMode::Dark) => Self::mossfire_dark(),
            (ThemeScheme::Mossfire, ThemeMode::Light) => Self::mossfire_light(),
            (ThemeScheme::NightSorbet, ThemeMode::Dark) => Self::night_sorbet_dark(),
            (ThemeScheme::NightSorbet, ThemeMode::Light) => Self::night_sorbet_light(),
            (ThemeScheme::GraphiteCode, ThemeMode::Dark) => Self::graphite_code_dark(),
            (ThemeScheme::GraphiteCode, ThemeMode::Light) => Self::graphite_code_light(),
            (ThemeScheme::FestivalCircuit, ThemeMode::Dark) => Self::festival_circuit_dark(),
            (ThemeScheme::FestivalCircuit, ThemeMode::Light) => Self::festival_circuit_light(),
        }
    }

    // ── Default ─────────────────────────────────────────────────

    fn default_dark() -> Self {
        Self {
            current_row_bg: Color::Rgb(24, 34, 46),
            highlight_row_bg: Color::Rgb(40, 48, 62),
            current_worktree_fg: Color::Rgb(244, 248, 255),
            dimmed: Color::Rgb(108, 112, 134),
            text: Color::Rgb(205, 214, 244),
            border: Color::Rgb(58, 74, 94),
            help_border: Color::Rgb(81, 104, 130),
            help_muted: Color::Rgb(112, 126, 144),
            header: Color::Rgb(180, 200, 220),
            keycap: Color::Rgb(249, 226, 175),
            info: Color::Rgb(120, 225, 213),
            success: Color::Rgb(166, 218, 149),
            warning: Color::Rgb(249, 226, 175),
            danger: Color::Rgb(237, 135, 150),
            accent: Color::Rgb(203, 166, 247),
        }
    }

    fn default_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(215, 230, 215),
            highlight_row_bg: Color::Rgb(200, 200, 210),
            current_worktree_fg: Color::Rgb(76, 79, 105),
            dimmed: Color::Rgb(140, 143, 161),
            text: Color::Rgb(76, 79, 105),
            border: Color::Rgb(160, 160, 175),
            help_border: Color::Rgb(130, 130, 160),
            help_muted: Color::Rgb(140, 143, 161),
            header: Color::Rgb(60, 75, 95),
            keycap: Color::Rgb(223, 142, 29),
            info: Color::Rgb(23, 146, 153),
            success: Color::Rgb(64, 160, 43),
            warning: Color::Rgb(223, 142, 29),
            danger: Color::Rgb(210, 15, 57),
            accent: Color::Rgb(136, 57, 239),
        }
    }

    // ── Emberforge ──────────────────────────────────────────────

    fn emberforge_dark() -> Self {
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

    fn emberforge_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(248, 235, 225),
            highlight_row_bg: Color::Rgb(238, 222, 210),
            current_worktree_fg: Color::Rgb(60, 40, 30),
            dimmed: Color::Rgb(155, 135, 120),
            text: Color::Rgb(70, 50, 38),
            border: Color::Rgb(195, 175, 160),
            help_border: Color::Rgb(175, 150, 130),
            help_muted: Color::Rgb(165, 145, 128),
            header: Color::Rgb(170, 100, 30),
            keycap: Color::Rgb(180, 130, 40),
            info: Color::Rgb(30, 130, 145),
            success: Color::Rgb(55, 140, 40),
            warning: Color::Rgb(180, 120, 20),
            danger: Color::Rgb(190, 60, 45),
            accent: Color::Rgb(175, 85, 55),
        }
    }

    // ── Glacier Signal ──────────────────────────────────────────

    fn glacier_signal_dark() -> Self {
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

    fn glacier_signal_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(230, 240, 248),
            highlight_row_bg: Color::Rgb(215, 228, 240),
            current_worktree_fg: Color::Rgb(20, 40, 65),
            dimmed: Color::Rgb(120, 140, 160),
            text: Color::Rgb(30, 50, 75),
            border: Color::Rgb(180, 195, 210),
            help_border: Color::Rgb(150, 175, 200),
            help_muted: Color::Rgb(140, 158, 175),
            header: Color::Rgb(40, 110, 190),
            keycap: Color::Rgb(30, 130, 180),
            info: Color::Rgb(20, 140, 160),
            success: Color::Rgb(40, 145, 110),
            warning: Color::Rgb(185, 130, 30),
            danger: Color::Rgb(200, 55, 55),
            accent: Color::Rgb(80, 90, 200),
        }
    }

    // ── Obsidian Pop ────────────────────────────────────────────

    fn obsidian_pop_dark() -> Self {
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

    fn obsidian_pop_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(245, 245, 250),
            highlight_row_bg: Color::Rgb(232, 232, 240),
            current_worktree_fg: Color::Rgb(15, 15, 25),
            dimmed: Color::Rgb(140, 140, 155),
            text: Color::Rgb(30, 30, 45),
            border: Color::Rgb(190, 190, 205),
            help_border: Color::Rgb(0, 160, 185),
            help_muted: Color::Rgb(130, 130, 150),
            header: Color::Rgb(190, 20, 140),
            keycap: Color::Rgb(170, 150, 0),
            info: Color::Rgb(0, 150, 190),
            success: Color::Rgb(50, 160, 0),
            warning: Color::Rgb(190, 125, 0),
            danger: Color::Rgb(210, 35, 35),
            accent: Color::Rgb(110, 50, 210),
        }
    }

    // ── Slate Garden ────────────────────────────────────────────

    fn slate_garden_dark() -> Self {
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

    fn slate_garden_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(240, 242, 245),
            highlight_row_bg: Color::Rgb(228, 232, 236),
            current_worktree_fg: Color::Rgb(40, 45, 55),
            dimmed: Color::Rgb(140, 145, 155),
            text: Color::Rgb(55, 62, 70),
            border: Color::Rgb(190, 195, 202),
            help_border: Color::Rgb(160, 170, 180),
            help_muted: Color::Rgb(150, 155, 165),
            header: Color::Rgb(70, 95, 120),
            keycap: Color::Rgb(140, 120, 60),
            info: Color::Rgb(60, 120, 118),
            success: Color::Rgb(75, 125, 65),
            warning: Color::Rgb(155, 125, 50),
            danger: Color::Rgb(150, 75, 80),
            accent: Color::Rgb(105, 90, 145),
        }
    }

    // ── Phosphor Arcade ─────────────────────────────────────────

    fn phosphor_arcade_dark() -> Self {
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

    fn phosphor_arcade_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(235, 248, 238),
            highlight_row_bg: Color::Rgb(220, 238, 224),
            current_worktree_fg: Color::Rgb(20, 50, 30),
            dimmed: Color::Rgb(120, 145, 118),
            text: Color::Rgb(30, 60, 38),
            border: Color::Rgb(180, 205, 185),
            help_border: Color::Rgb(140, 175, 148),
            help_muted: Color::Rgb(130, 155, 128),
            header: Color::Rgb(170, 120, 25),
            keycap: Color::Rgb(155, 130, 40),
            info: Color::Rgb(30, 140, 115),
            success: Color::Rgb(50, 150, 42),
            warning: Color::Rgb(175, 125, 20),
            danger: Color::Rgb(200, 55, 35),
            accent: Color::Rgb(45, 130, 195),
        }
    }

    // ── Lasergrid ───────────────────────────────────────────────

    fn lasergrid_dark() -> Self {
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

    fn lasergrid_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(242, 238, 252),
            highlight_row_bg: Color::Rgb(228, 222, 245),
            current_worktree_fg: Color::Rgb(25, 18, 50),
            dimmed: Color::Rgb(140, 128, 165),
            text: Color::Rgb(35, 28, 65),
            border: Color::Rgb(195, 185, 215),
            help_border: Color::Rgb(200, 30, 125),
            help_muted: Color::Rgb(145, 135, 168),
            header: Color::Rgb(0, 155, 170),
            keycap: Color::Rgb(130, 145, 0),
            info: Color::Rgb(0, 140, 175),
            success: Color::Rgb(35, 165, 55),
            warning: Color::Rgb(185, 115, 10),
            danger: Color::Rgb(205, 30, 85),
            accent: Color::Rgb(140, 35, 210),
        }
    }

    // ── Mossfire ────────────────────────────────────────────────

    fn mossfire_dark() -> Self {
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

    fn mossfire_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(242, 240, 232),
            highlight_row_bg: Color::Rgb(230, 228, 218),
            current_worktree_fg: Color::Rgb(35, 38, 28),
            dimmed: Color::Rgb(138, 140, 120),
            text: Color::Rgb(48, 52, 38),
            border: Color::Rgb(195, 192, 178),
            help_border: Color::Rgb(160, 165, 135),
            help_muted: Color::Rgb(145, 145, 128),
            header: Color::Rgb(95, 118, 45),
            keycap: Color::Rgb(160, 130, 50),
            info: Color::Rgb(40, 115, 128),
            success: Color::Rgb(55, 130, 45),
            warning: Color::Rgb(165, 118, 30),
            danger: Color::Rgb(160, 65, 45),
            accent: Color::Rgb(115, 75, 125),
        }
    }

    // ── Night Sorbet ────────────────────────────────────────────

    fn night_sorbet_dark() -> Self {
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

    fn night_sorbet_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(245, 240, 248),
            highlight_row_bg: Color::Rgb(232, 226, 240),
            current_worktree_fg: Color::Rgb(40, 35, 55),
            dimmed: Color::Rgb(148, 140, 162),
            text: Color::Rgb(50, 42, 65),
            border: Color::Rgb(200, 192, 215),
            help_border: Color::Rgb(170, 162, 195),
            help_muted: Color::Rgb(155, 148, 172),
            header: Color::Rgb(55, 120, 185),
            keycap: Color::Rgb(165, 135, 45),
            info: Color::Rgb(30, 145, 140),
            success: Color::Rgb(65, 150, 55),
            warning: Color::Rgb(185, 125, 55),
            danger: Color::Rgb(200, 65, 85),
            accent: Color::Rgb(130, 95, 200),
        }
    }

    // ── Graphite Code ───────────────────────────────────────────

    fn graphite_code_dark() -> Self {
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

    fn graphite_code_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(245, 246, 248),
            highlight_row_bg: Color::Rgb(234, 236, 240),
            current_worktree_fg: Color::Rgb(25, 28, 35),
            dimmed: Color::Rgb(140, 148, 158),
            text: Color::Rgb(40, 46, 55),
            border: Color::Rgb(195, 200, 208),
            help_border: Color::Rgb(170, 178, 188),
            help_muted: Color::Rgb(155, 162, 172),
            header: Color::Rgb(55, 62, 72),
            keycap: Color::Rgb(40, 45, 52),
            info: Color::Rgb(85, 95, 108),
            success: Color::Rgb(70, 78, 88),
            warning: Color::Rgb(100, 108, 118),
            danger: Color::Rgb(120, 128, 138),
            accent: Color::Rgb(60, 66, 76),
        }
    }

    // ── Festival Circuit ────────────────────────────────────────

    fn festival_circuit_dark() -> Self {
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

    fn festival_circuit_light() -> Self {
        Self {
            current_row_bg: Color::Rgb(240, 238, 252),
            highlight_row_bg: Color::Rgb(226, 224, 245),
            current_worktree_fg: Color::Rgb(25, 22, 55),
            dimmed: Color::Rgb(135, 132, 172),
            text: Color::Rgb(35, 30, 68),
            border: Color::Rgb(190, 195, 220),
            help_border: Color::Rgb(140, 155, 210),
            help_muted: Color::Rgb(145, 142, 180),
            header: Color::Rgb(15, 135, 180),
            keycap: Color::Rgb(165, 140, 0),
            info: Color::Rgb(20, 150, 135),
            success: Color::Rgb(45, 150, 30),
            warning: Color::Rgb(190, 105, 15),
            danger: Color::Rgb(205, 40, 75),
            accent: Color::Rgb(120, 50, 210),
        }
    }
}
