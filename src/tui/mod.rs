// Shared, reusable TUI infrastructure: a small library surface intended for use
// across multiple front-ends (dashboard, sidebar). Not every public item is
// consumed by every binary build yet, so dead-code analysis is relaxed here.
#![allow(dead_code)]

//! Shared TUI infrastructure reused across the dashboard and the sidebar.
//!
//! - `chord`   : terminal key chords (code + modifiers) with parse/display.
//! - `binding` : a single source of truth mapping chords -> actions per
//!   context, from which the keymap, command palette, and help overlay are all
//!   derived. Generic over each TUI's own action enum.
//! - `palette` : a reusable fuzzy command-palette state + renderer.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub mod binding;
pub mod chord;
pub mod dsl;
pub mod modal;
pub mod palette;

pub use binding::Binding;

/// A rect covering `percent_x` x `percent_y` of `area`, centered in it.
pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}
