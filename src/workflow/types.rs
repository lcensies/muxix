use std::path::PathBuf;

use crate::github::PrSummary;
use crate::prompt::Prompt;

/// Arguments for creating a worktree
pub struct CreateArgs<'a> {
    pub branch_name: &'a str,
    pub handle: &'a str,
    pub base_branch: Option<&'a str>,
    pub remote_branch: Option<&'a str>,
    pub prompt: Option<&'a Prompt>,
    pub options: SetupOptions,
    pub agent: Option<&'a str>,
}

/// Result of creating a worktree
pub struct CreateResult {
    pub worktree_path: PathBuf,
    pub branch_name: String,
    pub post_create_hooks_run: usize,
    pub base_branch: Option<String>,
    /// True if we switched to an existing window instead of creating a new one
    pub did_switch: bool,
}

/// Result of merging a worktree
pub struct MergeResult {
    pub branch_merged: String,
    pub main_branch: String,
    pub had_staged_changes: bool,
}

/// Result of removing a worktree
pub struct RemoveResult {
    pub branch_removed: String,
}

/// Result of cleanup operations
pub struct CleanupResult {
    pub tmux_window_killed: bool,
    pub worktree_removed: bool,
    pub local_branch_deleted: bool,
    /// The actual window name to close later (when running inside a duplicate window)
    pub window_to_close_later: Option<String>,
    /// Trash directory path to delete after window close (deferred to avoid race condition)
    pub trash_path_to_delete: Option<PathBuf>,
}

/// Options for setting up a worktree environment
#[derive(Debug, Clone)]
pub struct SetupOptions {
    pub run_hooks: bool,
    pub run_file_ops: bool,
    pub run_pane_commands: bool,
    pub prompt_file_path: Option<PathBuf>,
    /// If true, switch to the new tmux window when done; if false, leave it in the background.
    pub focus_window: bool,
}

impl SetupOptions {
    /// Create SetupOptions with all options enabled
    #[allow(dead_code)]
    pub fn all() -> Self {
        Self {
            run_hooks: true,
            run_file_ops: true,
            run_pane_commands: true,
            prompt_file_path: None,
            focus_window: true,
        }
    }

    /// Create SetupOptions with custom values
    pub fn new(run_hooks: bool, run_file_ops: bool, run_pane_commands: bool) -> Self {
        Self {
            run_hooks,
            run_file_ops,
            run_pane_commands,
            prompt_file_path: None,
            focus_window: true,
        }
    }

    /// Create SetupOptions with a prompt file
    #[allow(dead_code)]
    pub fn with_prompt(
        run_hooks: bool,
        run_file_ops: bool,
        run_pane_commands: bool,
        prompt_file_path: Option<PathBuf>,
    ) -> Self {
        Self {
            run_hooks,
            run_file_ops,
            run_pane_commands,
            prompt_file_path,
            focus_window: true,
        }
    }
}

/// List all worktrees with their status
pub struct WorktreeInfo {
    pub branch: String,
    pub path: PathBuf,
    pub has_tmux: bool,
    pub has_unmerged: bool,
    pub pr_info: Option<PrSummary>,
}
