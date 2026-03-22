//! TUI rendering logic for the dashboard.

mod dashboard;
mod diff;
mod format;
mod help;
pub mod theme;
pub mod worktree;

use ratatui::Frame;

use super::app::{App, ViewMode};

pub use self::dashboard::render_dashboard;
pub use self::diff::render_diff_view;
pub use self::help::{render_confirm_delete_worktree, render_confirm_kill, render_help};

/// Main UI entry point - renders the appropriate view based on app state.
pub fn ui(f: &mut Frame, app: &mut App) {
    // Render either dashboard or diff view based on view mode
    match &mut app.view_mode {
        ViewMode::Dashboard => render_dashboard(f, app),
        ViewMode::Diff(diff_view) => render_diff_view(f, diff_view, &app.palette),
    }

    // Render overlays on top
    if app.show_help {
        render_help(f, app);
    } else if app.pending_kill_pane_id.is_some() {
        render_confirm_kill(f, app);
    } else if app.pending_delete_worktree.is_some() {
        render_confirm_delete_worktree(f, app);
    }
}
