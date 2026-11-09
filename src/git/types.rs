use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct RemoteBranchSpec {
    pub remote: String,
    pub branch: String,
}

#[derive(Debug, Clone)]
pub struct ForkBranchSpec {
    pub owner: String,
    pub branch: String,
}

/// Custom error type for worktree not found
#[derive(Debug, thiserror::Error)]
#[error("Worktree not found: {0}")]
pub struct WorktreeNotFound(pub String);

/// Git status information for a worktree
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitStatus {
    /// Commits ahead of upstream
    pub ahead: usize,
    /// Commits behind upstream
    pub behind: usize,
    /// Branch has conflicts when merging with base
    pub has_conflict: bool,
    /// Has uncommitted changes (staged or unstaged)
    pub is_dirty: bool,
    /// Lines added in committed changes only (base...HEAD)
    pub lines_added: usize,
    /// Lines removed in committed changes only (base...HEAD)
    pub lines_removed: usize,
    /// Lines added in uncommitted changes only (working tree + untracked)
    #[serde(default)]
    pub uncommitted_added: usize,
    /// Lines removed in uncommitted changes only (working tree)
    #[serde(default)]
    pub uncommitted_removed: usize,
    /// Timestamp when this status was cached (UNIX seconds)
    #[serde(default)]
    pub cached_at: Option<u64>,
    /// The base branch used for comparison (e.g., "main")
    #[serde(default)]
    pub base_branch: String,
    /// The branch name for this worktree (None if detached HEAD)
    #[serde(default)]
    pub branch: Option<String>,
    /// Whether the branch has an upstream tracking branch
    #[serde(default)]
    pub has_upstream: bool,
    /// Whether a rebase is currently in progress
    #[serde(default)]
    pub is_rebasing: bool,
    /// HEAD's object id when this status was computed. `None` for an empty
    /// repository, a detached HEAD, or an entry written before this field
    /// existed; a refresh with no HEAD oid cannot skip any work.
    #[serde(default)]
    pub head_oid: Option<String>,
    /// The base branch's object id when this status was computed.
    #[serde(default)]
    pub base_oid: Option<String>,
    /// Whether `base_branch` came from the caller's configured `main_branch`
    /// rather than from the branch's own `muxix-base` or the repository's
    /// default branch. Only then does the base depend on who computed the
    /// status, so only then can another process's reply disagree with ours.
    #[serde(default)]
    pub base_from_main_branch: bool,
}
