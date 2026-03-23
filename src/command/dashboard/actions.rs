//! Action enum and dispatcher for dashboard key handling.

use super::app::{App, DashboardTab, ViewMode};
use super::diff_ops::DiffOps;

/// All possible actions in the dashboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // Global actions
    ShowHelp,
    Quit,

    // Dashboard navigation
    Next,
    Previous,
    JumpToSelected,
    JumpToIndex(usize),
    JumpToLast,
    PeekSelected,

    // Tab switching
    SwitchTab,

    // Dashboard commands
    CycleColorScheme,
    CycleSortMode,
    ToggleScopeFilter,
    ToggleStaleFilter,
    EnterInputMode,
    ExitInputMode,
    ScrollPreviewUp,
    ScrollPreviewDown,
    IncreasePreviewSize,
    DecreasePreviewSize,
    LoadWipDiff,
    SendCommitDashboard,
    TriggerMergeDashboard,
    KillSelected,

    // Input mode
    SendKey(String),

    // Diff view navigation
    CloseDiff,
    ScrollUp,
    ScrollDown,
    ScrollPageUp,
    ScrollPageDown,
    ToggleDiffType,
    EnterPatchMode,
    SendCommitDiff,
    TriggerMergeDiff,

    // Patch mode
    StageAndNext,
    SkipHunk,
    UndoStagedHunk,
    SplitHunk,
    StartComment,
    PrevHunk,
    NextHunk,
    ExitPatchMode,

    // Worktree view
    WorktreeNext,
    WorktreePrevious,
    WorktreeJumpToIndex(usize),
    RemoveSelectedWorktree,
    StartSweep,
    CycleWorktreeSortMode,
    JumpToSelectedWorktree,

    // Filter mode
    EnterFilterMode,
    AcceptFilter,
    ClearFilter,
    FilterAppendChar(char),
    FilterDeleteChar,

    // Comment input
    CancelComment,
    SendComment,
    DeleteChar,
    AppendChar(char),
}

/// Apply an action to the app state.
/// Returns true if preview should be refreshed immediately.
pub fn apply_action(app: &mut App, action: Action) -> bool {
    match action {
        // Global
        Action::ShowHelp => {
            app.show_help = true;
            false
        }
        Action::Quit => {
            match app.active_tab {
                DashboardTab::Agents => {
                    if !app.filter_text.is_empty() {
                        app.filter_text.clear();
                        app.apply_filters();
                    } else {
                        app.should_quit = true;
                    }
                }
                DashboardTab::Worktrees => {
                    if !app.worktree_filter_text.is_empty() {
                        app.worktree_filter_text.clear();
                        app.trigger_worktree_refetch();
                    } else {
                        app.should_quit = true;
                    }
                }
            }
            false
        }

        // Dashboard navigation
        Action::Next => {
            app.next();
            false
        }
        Action::Previous => {
            app.previous();
            false
        }
        Action::JumpToSelected => {
            app.jump_to_selected();
            false
        }
        Action::JumpToIndex(idx) => {
            app.jump_to_index(idx);
            false
        }
        Action::JumpToLast => {
            app.jump_to_last();
            false
        }
        Action::PeekSelected => {
            app.peek_selected();
            false
        }

        // Dashboard commands
        Action::CycleColorScheme => {
            app.cycle_color_scheme();
            false
        }
        Action::CycleSortMode => {
            app.cycle_sort_mode();
            false
        }
        Action::ToggleScopeFilter => {
            app.toggle_scope_mode();
            false
        }
        Action::ToggleStaleFilter => {
            app.toggle_stale_filter();
            false
        }
        Action::EnterInputMode => {
            if app.table_state.selected().is_some() && !app.agents.is_empty() {
                app.input_mode = true;
            }
            false
        }
        Action::ExitInputMode => {
            app.input_mode = false;
            false
        }
        Action::ScrollPreviewUp => {
            app.scroll_preview_up(app.preview_height, app.preview_line_count);
            false
        }
        Action::ScrollPreviewDown => {
            app.scroll_preview_down(app.preview_height, app.preview_line_count);
            false
        }
        Action::IncreasePreviewSize => {
            app.increase_preview_size();
            false
        }
        Action::DecreasePreviewSize => {
            app.decrease_preview_size();
            false
        }
        Action::LoadWipDiff => {
            app.load_diff(false);
            false
        }
        Action::SendCommitDashboard => {
            app.send_commit_to_selected();
            false
        }
        Action::TriggerMergeDashboard => {
            app.trigger_merge_for_selected();
            false
        }
        Action::KillSelected => {
            app.kill_selected();
            false
        }

        // Tab switching
        Action::SwitchTab => {
            app.switch_tab();
            false
        }

        // Worktree view
        Action::WorktreeNext => {
            app.worktree_next();
            false
        }
        Action::WorktreePrevious => {
            app.worktree_previous();
            false
        }
        Action::WorktreeJumpToIndex(idx) => {
            app.worktree_jump_to_index(idx);
            false
        }
        Action::RemoveSelectedWorktree => {
            app.remove_selected_worktree();
            false
        }
        Action::StartSweep => {
            app.start_sweep();
            false
        }
        Action::CycleWorktreeSortMode => {
            app.cycle_worktree_sort_mode();
            false
        }
        Action::JumpToSelectedWorktree => {
            app.jump_to_selected_worktree();
            false
        }

        // Filter mode (tab-aware)
        Action::EnterFilterMode => {
            match app.active_tab {
                DashboardTab::Agents => app.filter_active = true,
                DashboardTab::Worktrees => app.worktree_filter_active = true,
            }
            false
        }
        Action::AcceptFilter => {
            match app.active_tab {
                DashboardTab::Agents => app.filter_active = false,
                DashboardTab::Worktrees => app.worktree_filter_active = false,
            }
            false
        }
        Action::ClearFilter => {
            match app.active_tab {
                DashboardTab::Agents => {
                    app.filter_active = false;
                    app.filter_text.clear();
                    app.apply_filters();
                }
                DashboardTab::Worktrees => {
                    app.worktree_filter_active = false;
                    app.worktree_filter_text.clear();
                    // Trigger re-fetch to restore full list
                    app.trigger_worktree_refetch();
                }
            }
            false
        }
        Action::FilterAppendChar(c) => {
            match app.active_tab {
                DashboardTab::Agents => {
                    app.filter_text.push(c);
                    app.apply_filters();
                }
                DashboardTab::Worktrees => {
                    app.worktree_filter_text.push(c);
                    // Trigger re-fetch to apply filter
                    app.trigger_worktree_refetch();
                }
            }
            false
        }
        Action::FilterDeleteChar => {
            match app.active_tab {
                DashboardTab::Agents => {
                    app.filter_text.pop();
                    app.apply_filters();
                }
                DashboardTab::Worktrees => {
                    app.worktree_filter_text.pop();
                    // Trigger re-fetch to apply filter
                    app.trigger_worktree_refetch();
                }
            }
            false
        }

        // Input mode
        Action::SendKey(key) => {
            app.send_key_to_selected(&key);
            app.refresh_preview();
            true // Signal that preview was refreshed
        }

        // Diff view
        Action::CloseDiff => {
            app.close_diff();
            false
        }
        Action::ScrollUp => {
            if let ViewMode::Diff(ref mut diff) = app.view_mode {
                diff.scroll_up();
            }
            false
        }
        Action::ScrollDown => {
            if let ViewMode::Diff(ref mut diff) = app.view_mode {
                diff.scroll_down();
            }
            false
        }
        Action::ScrollPageUp => {
            if let ViewMode::Diff(ref mut diff) = app.view_mode {
                diff.scroll_page_up();
            }
            false
        }
        Action::ScrollPageDown => {
            if let ViewMode::Diff(ref mut diff) = app.view_mode {
                diff.scroll_page_down();
            }
            false
        }
        Action::ToggleDiffType => {
            let is_branch_diff = if let ViewMode::Diff(ref diff) = app.view_mode {
                diff.is_branch_diff
            } else {
                false
            };
            app.load_diff(!is_branch_diff);
            false
        }
        Action::EnterPatchMode => {
            app.enter_patch_mode();
            false
        }
        Action::SendCommitDiff => {
            app.send_commit_to_agent();
            false
        }
        Action::TriggerMergeDiff => {
            app.trigger_merge();
            false
        }

        // Patch mode
        Action::StageAndNext => {
            app.stage_and_next();
            false
        }
        Action::SkipHunk => {
            app.skip_hunk();
            false
        }
        Action::UndoStagedHunk => {
            app.undo_staged_hunk();
            false
        }
        Action::SplitHunk => {
            app.split_current_hunk();
            false
        }
        Action::StartComment => {
            if let ViewMode::Diff(ref mut diff) = app.view_mode {
                diff.comment_input = Some(String::new());
            }
            false
        }
        Action::PrevHunk => {
            app.prev_hunk();
            false
        }
        Action::NextHunk => {
            let _ = app.next_hunk();
            false
        }
        Action::ExitPatchMode => {
            app.exit_patch_mode();
            false
        }

        // Comment input
        Action::CancelComment => {
            if let ViewMode::Diff(ref mut diff) = app.view_mode {
                diff.comment_input = None;
            }
            false
        }
        Action::SendComment => {
            app.send_hunk_comment();
            false
        }
        Action::DeleteChar => {
            if let ViewMode::Diff(ref mut diff) = app.view_mode
                && let Some(ref mut input) = diff.comment_input
            {
                input.pop();
            }
            false
        }
        Action::AppendChar(c) => {
            if let ViewMode::Diff(ref mut diff) = app.view_mode
                && let Some(ref mut input) = diff.comment_input
            {
                input.push(c);
            }
            false
        }
    }
}
