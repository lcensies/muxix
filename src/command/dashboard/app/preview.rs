//! Preview pane management for the dashboard.

use super::super::settings::save_preview_size;
use super::App;

/// Number of lines to capture from the agent's terminal for preview (scrollable history)
pub const PREVIEW_LINES: u16 = 200;

impl App {
    /// Update the preview for the currently selected agent.
    /// Only fetches if the selection has changed or preview is stale.
    pub fn update_preview(&mut self) {
        if !self.mux.supports_preview() {
            return;
        }
        let current_pane_id = self
            .table_state
            .selected()
            .and_then(|idx| self.agents.get(idx))
            .map(|agent| agent.pane_id.clone());

        // Only fetch if selection changed
        if current_pane_id != self.preview_pane_id {
            self.preview_pane_id = current_pane_id.clone();
            self.preview = current_pane_id
                .as_ref()
                .and_then(|pane_id| self.mux.capture_pane(pane_id, PREVIEW_LINES));
            // Reset scroll position when selection changes
            self.preview_scroll = None;
        }
    }

    /// Force refresh the preview (used on periodic refresh)
    pub fn refresh_preview(&mut self) {
        if !self.mux.supports_preview() {
            return;
        }
        self.preview = self
            .preview_pane_id
            .as_ref()
            .and_then(|pane_id| self.mux.capture_pane(pane_id, PREVIEW_LINES));
    }

    /// Scroll preview up (toward older content). Returns the amount to scroll by.
    pub fn scroll_preview_up(&mut self, visible_height: u16, total_lines: u16) {
        let max_scroll = total_lines.saturating_sub(visible_height);
        let current = self.preview_scroll.unwrap_or(max_scroll);
        let half_page = visible_height / 2;
        self.preview_scroll = Some(current.saturating_sub(half_page));
    }

    /// Scroll preview down (toward newer content).
    pub fn scroll_preview_down(&mut self, visible_height: u16, total_lines: u16) {
        let max_scroll = total_lines.saturating_sub(visible_height);
        let current = self.preview_scroll.unwrap_or(max_scroll);
        let half_page = visible_height / 2;
        let new_scroll = (current + half_page).min(max_scroll);
        // If at or past max, return to auto-scroll mode
        if new_scroll >= max_scroll {
            self.preview_scroll = None;
        } else {
            self.preview_scroll = Some(new_scroll);
        }
    }

    /// Increase preview size by 10% (max 90%)
    pub fn increase_preview_size(&mut self) {
        self.preview_size = (self.preview_size + 10).min(90);
        save_preview_size(self.preview_size);
    }

    /// Decrease preview size by 10% (min 10%)
    pub fn decrease_preview_size(&mut self) {
        self.preview_size = self.preview_size.saturating_sub(10).max(10);
        save_preview_size(self.preview_size);
    }
}
