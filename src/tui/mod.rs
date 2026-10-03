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

pub mod binding;
pub mod chord;
pub mod dsl;
pub mod palette;

pub use binding::Binding;
