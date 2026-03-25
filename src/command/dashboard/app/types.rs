//! Data types for the dashboard application state.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::git::GitStatus;
use crate::github::{PrListEntry, PrSummary};
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
    /// Result of fetching open PRs for the add-worktree modal
    AddWorktreePrList(u64, Result<Vec<PrListEntry>, String>),
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

/// Mode for the add-worktree modal.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum AddWorktreeMode {
    #[default]
    Branch,
    Pr,
}

/// Loading state for the PR list in the add-worktree modal.
pub enum PrListState {
    Loading,
    Loaded { prs: Vec<PrListEntry> },
    Error { message: String },
}

/// State for the add-worktree modal.
pub struct AddWorktreeState {
    /// All local branches (fetched once when modal opens).
    pub branches: Vec<String>,
    /// Branches that already have worktrees (cannot create another).
    pub occupied_branches: std::collections::HashSet<String>,
    /// Cursor position: 0 = "Create new", 1..N = filtered branch index.
    pub cursor: usize,
    /// Filter text (doubles as new branch name).
    pub filter: String,
    /// Original typed prefix preserved during Tab cycling (cleared on typing).
    pub tab_prefix: Option<String>,
    /// Base branch for new worktree creation (defaults to main/master).
    pub base_branch: String,
    /// Whether the base branch field is being edited (Ctrl+b toggles).
    pub editing_base: bool,
    /// Filter/input text for the base branch field.
    pub base_filter: String,
    /// Tab prefix for base branch cycling.
    pub base_tab_prefix: Option<String>,
    pub repo_path: PathBuf,
    /// Current mode: Branch picker or PR list.
    pub mode: AddWorktreeMode,
    /// PR list state (loaded async when switching to PR mode).
    pub pr_list: Option<PrListState>,
    /// Monotonic counter to discard stale PR list results.
    pub pr_request_counter: u64,
}

impl AddWorktreeState {
    /// Return indices into `branches` that match the current filter.
    /// During tab cycling, uses the original typed prefix so all matches stay visible.
    /// Available branches appear first, occupied branches (already have worktrees) last.
    pub fn filtered(&self) -> Vec<usize> {
        let text = self.tab_prefix.as_deref().unwrap_or(&self.filter);
        let matches: Vec<usize> = if text.is_empty() {
            (0..self.branches.len()).collect()
        } else {
            let lower = text.to_lowercase();
            self.branches
                .iter()
                .enumerate()
                .filter(|(_, b)| b.to_lowercase().contains(&lower))
                .map(|(i, _)| i)
                .collect()
        };

        // Sort: available branches first, occupied last
        let mut available: Vec<usize> = Vec::new();
        let mut occupied: Vec<usize> = Vec::new();
        for idx in matches {
            if self.occupied_branches.contains(&self.branches[idx]) {
                occupied.push(idx);
            } else {
                available.push(idx);
            }
        }
        available.extend(occupied);
        available
    }

    /// Number of selectable (non-occupied) entries in `filtered()`.
    /// Since `filtered()` places available branches before occupied ones,
    /// this is the count of leading non-occupied entries.
    pub fn selectable_count(&self) -> usize {
        let filtered = self.filtered();
        filtered
            .iter()
            .take_while(|&&idx| !self.occupied_branches.contains(&self.branches[idx]))
            .count()
    }

    /// If the filter text looks like a PR number, return it.
    /// Matches "#123" or bare "123" (only digits).
    pub fn detected_pr_number(&self) -> Option<u32> {
        let text = self.filter.trim();
        let digits = text.strip_prefix('#').unwrap_or(text);
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            digits.parse().ok()
        } else {
            None
        }
    }

    /// Return indices into the PR list that match the current filter.
    pub fn filtered_prs(&self) -> Vec<usize> {
        let prs = match &self.pr_list {
            Some(PrListState::Loaded { prs, .. }) => prs,
            _ => return Vec::new(),
        };
        if self.filter.is_empty() {
            return (0..prs.len()).collect();
        }
        let lower = self.filter.to_lowercase();
        prs.iter()
            .enumerate()
            .filter(|(_, pr)| {
                pr.title.to_lowercase().contains(&lower)
                    || pr.head_ref_name.to_lowercase().contains(&lower)
                    || pr.number.to_string().contains(&lower)
                    || pr.author.to_lowercase().contains(&lower)
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
