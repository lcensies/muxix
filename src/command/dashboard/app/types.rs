//! Data types for the dashboard application state.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::git::GitStatus;
use crate::github::PrSummary;
use crate::workflow::types::WorktreeInfo;

use super::super::diff::DiffView;

/// Unified event type for the dashboard event loop.
/// All background threads and the input thread send events through a single channel.
pub enum AppEvent {
    /// Terminal input event (from dedicated input thread)
    Terminal(crossterm::event::Event),
    /// Git status update for a worktree path
    GitStatus(PathBuf, GitStatus),
    /// PR status update for a repo root
    PrStatus(PathBuf, HashMap<String, PrSummary>),
    /// Full worktree list from background fetch
    WorktreeList(Vec<WorktreeInfo>),
    /// Git log preview for a worktree path
    WorktreeLog(PathBuf, String),
    /// Result of a background add-worktree operation
    AddWorktreeResult(Result<String, String>),
}

/// Which tab is active in the dashboard
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DashboardTab {
    #[default]
    Agents,
    Worktrees,
}

/// Current view mode of the dashboard
#[derive(Debug, Default, PartialEq)]
pub enum ViewMode {
    #[default]
    Dashboard,
    Diff(Box<DiffView>),
}

/// A candidate worktree for bulk sweep cleanup.
pub struct SweepCandidate {
    pub handle: String,
    pub path: PathBuf,
    pub reason: SweepReason,
    pub is_dirty: bool,
    pub selected: bool,
}

/// Why a worktree is a sweep candidate.
#[derive(Clone)]
pub enum SweepReason {
    PrMerged,
    PrClosed,
    UpstreamGone,
    MergedLocally,
}

impl SweepReason {
    pub fn label(&self) -> &'static str {
        match self {
            SweepReason::PrMerged => "PR merged",
            SweepReason::PrClosed => "PR closed",
            SweepReason::UpstreamGone => "upstream gone",
            SweepReason::MergedLocally => "merged locally",
        }
    }
}

/// State for the bulk sweep modal.
pub struct SweepState {
    pub candidates: Vec<SweepCandidate>,
    pub cursor: usize,
}

/// An entry in the project picker.
pub struct ProjectEntry {
    pub name: String,
    pub path: PathBuf,
}

/// State for the project picker modal.
pub struct ProjectPicker {
    pub projects: Vec<ProjectEntry>,
    pub cursor: usize,
    pub filter: String,
    pub current_name: Option<String>,
}

impl ProjectPicker {
    /// Return indices into `projects` that match the current filter.
    pub fn filtered(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.projects.len()).collect();
        }
        let lower = self.filter.to_lowercase();
        self.projects
            .iter()
            .enumerate()
            .filter(|(_, p)| p.name.to_lowercase().contains(&lower))
            .map(|(i, _)| i)
            .collect()
    }
}

/// State for the base branch picker modal.
pub struct BaseBranchPicker {
    pub branches: Vec<String>,
    pub cursor: usize,
    pub filter: String,
    /// Current base branch of the selected worktree (highlighted in picker)
    pub current_base: Option<String>,
    /// Branch name of the worktree being edited
    pub worktree_branch: String,
    /// Path to the worktree's repo (for running git commands)
    pub repo_path: PathBuf,
}

impl BaseBranchPicker {
    /// Return indices into `branches` that match the current filter.
    pub fn filtered(&self) -> Vec<usize> {
        if self.filter.is_empty() {
            return (0..self.branches.len()).collect();
        }
        let lower = self.filter.to_lowercase();
        self.branches
            .iter()
            .enumerate()
            .filter(|(_, b)| b.to_lowercase().contains(&lower))
            .map(|(i, _)| i)
            .collect()
    }
}

/// Phase of the add-worktree modal.
pub enum AddWorktreePhase {
    /// Unified picker: filter text doubles as new branch name, existing branches shown below.
    /// Cursor 0 = "Create new branch", cursor 1..N = existing branch matches.
    SelectOrCreate,
    /// Base branch picker (only shown when creating a new branch).
    BaseBranch,
}

/// State for the add-worktree modal.
pub struct AddWorktreeState {
    pub phase: AddWorktreePhase,
    /// The confirmed branch name (set when advancing from SelectOrCreate to BaseBranch).
    pub name: String,
    /// All local branches (fetched once when modal opens).
    pub branches: Vec<String>,
    /// Branches that already have worktrees (cannot create another).
    pub occupied_branches: std::collections::HashSet<String>,
    /// Cursor position: 0 = "Create new", 1..N = filtered branch index.
    pub cursor: usize,
    /// Filter text (doubles as new branch name in SelectOrCreate phase).
    pub filter: String,
    /// Original typed prefix preserved during Tab cycling (cleared on typing).
    pub tab_prefix: Option<String>,
    pub repo_path: PathBuf,
}

impl AddWorktreeState {
    /// Return indices into `branches` that match the current filter.
    /// During tab cycling, uses the original typed prefix so all matches stay visible.
    /// Occupied branches (already have worktrees) are excluded.
    pub fn filtered(&self) -> Vec<usize> {
        let text = self.tab_prefix.as_deref().unwrap_or(&self.filter);
        if text.is_empty() {
            return self
                .branches
                .iter()
                .enumerate()
                .filter(|(_, b)| !self.occupied_branches.contains(*b))
                .map(|(i, _)| i)
                .collect();
        }
        let lower = text.to_lowercase();
        self.branches
            .iter()
            .enumerate()
            .filter(|(_, b)| {
                b.to_lowercase().contains(&lower) && !self.occupied_branches.contains(*b)
            })
            .map(|(i, _)| i)
            .collect()
    }
}

/// Plan for a pending worktree removal (shown in confirmation modal).
pub struct RemovePlan {
    pub handle: String,
    pub path: PathBuf,
    pub is_dirty: bool,
    pub is_unmerged: bool,
    pub keep_branch: bool,
    pub force_armed: bool,
}
