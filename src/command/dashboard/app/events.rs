//! Event dispatching for background events.

use super::App;
use super::types::{AppEvent, PrListState};

impl App {
    /// Apply a background event to app state.
    /// Called from the main loop when an AppEvent arrives on the unified channel.
    pub fn apply_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Terminal(_) => {} // handled separately in main loop
            AppEvent::GitStatus(path, status) => {
                self.git_statuses.insert(path, status);
            }
            AppEvent::PrStatus(repo_root, prs) => {
                // Clear stale state when a repo has no PRs, otherwise update
                if prs.is_empty() {
                    self.pr_statuses.remove(&repo_root);
                } else {
                    self.pr_statuses.insert(repo_root, prs);
                }
                // Re-apply worktree filters to merge new PR data
                if !self.all_worktrees.is_empty() {
                    self.apply_worktree_filters();
                }
            }
            AppEvent::WorktreeList(worktrees) => {
                let needs_pr_fetch = self.all_worktrees.is_empty() && !worktrees.is_empty();
                self.all_worktrees = worktrees;
                self.apply_worktree_filters();

                // Force a PR re-fetch on initial load or after project switch
                // (confirm_project_picker clears all_worktrees, so this fires)
                if needs_pr_fetch {
                    self.last_pr_fetch = std::time::Instant::now() - super::PR_FETCH_INTERVAL;
                }
            }
            AppEvent::WorktreeLog(path, log) => {
                if self.worktree_preview_path.as_ref() == Some(&path) {
                    self.worktree_preview = Some(log);
                }
            }
            AppEvent::AddWorktreeResult(result) => {
                self.handle_add_worktree_result(result);
            }
            AppEvent::AddWorktreePrList(request_id, result) => {
                if let Some(ref mut state) = self.pending_add_worktree
                    && request_id == state.pr_request_counter
                {
                    state.pr_list = Some(match result {
                        Ok(prs) => PrListState::Loaded { prs },
                        Err(msg) => PrListState::Error { message: msg },
                    });
                }
            }
        }
    }
}
