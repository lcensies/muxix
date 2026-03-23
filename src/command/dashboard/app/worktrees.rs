//! Worktree tab: navigation, removal, sweep, project picker, and preview.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::git;
use crate::workflow;

use super::super::agent;
use super::super::sort::WorktreeSortMode;
use super::App;
use super::types::*;

impl App {
    /// Reset the worktree fetch timer to trigger an immediate refetch
    pub fn trigger_worktree_refetch(&mut self) {
        self.last_worktree_fetch = std::time::Instant::now() - Duration::from_secs(60);
    }

    /// Switch between Agents and Worktrees tabs
    pub fn switch_tab(&mut self) {
        self.active_tab = match self.active_tab {
            DashboardTab::Agents => DashboardTab::Worktrees,
            DashboardTab::Worktrees => DashboardTab::Agents,
        };
        if self.active_tab == DashboardTab::Worktrees {
            // Trigger immediate fetch on switch
            self.last_worktree_fetch = std::time::Instant::now();
            self.spawn_worktree_fetch();
        }
    }

    /// Spawn background thread to fetch worktree list
    pub(super) fn spawn_worktree_fetch(&self) {
        if self
            .is_worktree_fetching
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let tx = self.event_tx.clone();
        let is_fetching = self.is_worktree_fetching.clone();
        let config = self.config.clone();
        let mux = self.mux.clone();
        let repo_override = self
            .worktree_project_override
            .as_ref()
            .map(|(_, p)| p.clone());

        std::thread::spawn(move || {
            struct ResetFlag(Arc<AtomicBool>);
            impl Drop for ResetFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = ResetFlag(is_fetching);

            // fetch_pr_status=false: the dashboard fetches PR status separately,
            // and workflow::list's spinner would corrupt the TUI output
            if let Ok(worktrees) =
                workflow::list_in(&config, mux.as_ref(), false, &[], repo_override.as_deref())
            {
                let _ = tx.send(AppEvent::WorktreeList(worktrees));
            }
        });
    }

    /// Cycle to the next worktree sort mode.
    pub fn cycle_worktree_sort_mode(&mut self) {
        self.worktree_sort_mode = self.worktree_sort_mode.next();
        self.worktree_sort_mode.save();
        self.apply_worktree_filters();
    }

    /// Sort worktrees according to the current sort mode.
    fn sort_worktrees(&mut self) {
        match self.worktree_sort_mode {
            WorktreeSortMode::Natural => {} // Keep original order from git
            WorktreeSortMode::Age => {
                self.worktrees
                    .sort_by(|a, b| b.created_at.cmp(&a.created_at));
            }
        }
    }

    /// Apply filter text to worktree list and restore selection
    pub(super) fn apply_worktree_filters(&mut self) {
        // Reset from baseline
        self.worktrees = self.all_worktrees.clone();

        // Merge PR data from dashboard's own PR fetching into worktrees
        // (workflow::list is called with fetch_pr_status=false to avoid spinner)
        if !self.pr_statuses.is_empty() {
            for wt in &mut self.worktrees {
                if wt.pr_info.is_some() || wt.is_main {
                    continue;
                }
                // Search all repo roots for a matching branch
                for prs in self.pr_statuses.values() {
                    if let Some(pr) = prs.get(&wt.branch) {
                        wt.pr_info = Some(pr.clone());
                        break;
                    }
                }
            }
        }

        // Apply name filter
        if !self.worktree_filter_text.is_empty() {
            let filter = self.worktree_filter_text.to_lowercase();
            self.worktrees.retain(|w| {
                let handle = w.handle.to_lowercase();
                handle.contains(&filter) || w.branch.to_lowercase().contains(&filter)
            });
        }

        // Sort after filtering
        self.sort_worktrees();

        // Restore selection by path
        if let Some(ref path) = self.selected_worktree_path {
            if let Some(idx) = self.worktrees.iter().position(|w| &w.path == path) {
                self.worktree_table_state.select(Some(idx));
            } else {
                self.selected_worktree_path = None;
                if self.worktrees.is_empty() {
                    self.worktree_table_state.select(None);
                } else {
                    self.worktree_table_state.select(Some(0));
                }
            }
        } else if !self.worktrees.is_empty() && self.worktree_table_state.selected().is_none() {
            self.worktree_table_state.select(Some(0));
            self.selected_worktree_path = self.worktrees.first().map(|w| w.path.clone());
        }

        self.update_worktree_preview();
    }

    pub fn worktree_next(&mut self) {
        if self.worktrees.is_empty() {
            return;
        }
        let i = self.worktree_table_state.selected().unwrap_or(0);
        let next = if i >= self.worktrees.len() - 1 {
            0
        } else {
            i + 1
        };
        self.worktree_table_state.select(Some(next));
        self.selected_worktree_path = self.worktrees.get(next).map(|w| w.path.clone());
        self.update_worktree_preview();
    }

    pub fn worktree_previous(&mut self) {
        if self.worktrees.is_empty() {
            return;
        }
        let i = self.worktree_table_state.selected().unwrap_or(0);
        let prev = if i == 0 {
            self.worktrees.len() - 1
        } else {
            i - 1
        };
        self.worktree_table_state.select(Some(prev));
        self.selected_worktree_path = self.worktrees.get(prev).map(|w| w.path.clone());
        self.update_worktree_preview();
    }

    pub fn worktree_jump_to_index(&mut self, index: usize) {
        if index < self.worktrees.len() {
            self.worktree_table_state.select(Some(index));
            self.selected_worktree_path = self.worktrees.get(index).map(|w| w.path.clone());
            self.jump_to_selected_worktree();
        }
    }

    /// Show the remove confirmation modal for the selected worktree.
    /// Always shows the modal (even for clean worktrees). Skips main worktree.
    pub fn remove_selected_worktree(&mut self) {
        let Some(selected) = self.worktree_table_state.selected() else {
            return;
        };
        let Some(worktree) = self.worktrees.get(selected) else {
            return;
        };

        // Block removal of main worktree
        if worktree.is_main {
            return;
        }

        let is_dirty = git::has_uncommitted_changes(&worktree.path).unwrap_or(false);

        self.pending_remove = Some(RemovePlan {
            handle: worktree.handle.clone(),
            path: worktree.path.clone(),
            is_dirty,
            is_unmerged: worktree.has_unmerged,
            keep_branch: false,
            force_armed: false,
        });
    }

    /// Toggle keep-branch in the pending remove plan.
    pub fn toggle_remove_keep_branch(&mut self) {
        if let Some(ref mut plan) = self.pending_remove {
            plan.keep_branch = !plan.keep_branch;
        }
    }

    /// Arm force mode for dirty worktree removal.
    pub fn arm_remove_force(&mut self) {
        if let Some(ref mut plan) = self.pending_remove
            && plan.is_dirty
        {
            plan.force_armed = true;
        }
    }

    /// Execute the pending remove confirmation.
    pub fn confirm_remove(&mut self) {
        let Some(plan) = self.pending_remove.take() else {
            return;
        };

        // Dirty worktrees require force to be armed
        if plan.is_dirty && !plan.force_armed {
            self.pending_remove = Some(plan);
            return;
        }

        self.do_remove_worktree(&plan.path, plan.keep_branch);
    }

    fn do_remove_worktree(&mut self, path: &Path, keep_branch: bool) {
        let handle = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();

        let Ok(ctx) = workflow::WorkflowContext::new(self.config.clone(), self.mux.clone(), None)
        else {
            return;
        };

        // force=true because user confirmed via modal
        if workflow::remove(&handle, true, keep_branch, &ctx).is_ok() {
            self.worktrees.retain(|w| w.path != *path);

            if self.worktrees.is_empty() {
                self.worktree_table_state.select(None);
                self.selected_worktree_path = None;
            } else {
                let idx = self.worktree_table_state.selected().unwrap_or(0);
                let new_idx = idx.min(self.worktrees.len() - 1);
                self.worktree_table_state.select(Some(new_idx));
                self.selected_worktree_path = self.worktrees.get(new_idx).map(|w| w.path.clone());
            }
        }
    }

    /// Close the mux window/session for the selected worktree without removing it.
    pub fn close_selected_worktree_window(&mut self) {
        let Some(selected) = self.worktree_table_state.selected() else {
            return;
        };
        let Some(worktree) = self.worktrees.get(selected) else {
            return;
        };

        if worktree.is_main || !worktree.has_mux_window {
            return;
        }

        let prefix = self.config.window_prefix();
        let full_name = crate::multiplexer::util::prefixed(prefix, &worktree.handle);
        let _ = crate::multiplexer::handle::MuxHandle::kill_full(
            self.mux.as_ref(),
            worktree.mode,
            &full_name,
        );
        self.trigger_worktree_refetch();
    }

    /// Build the sweep candidate list and open the sweep modal.
    /// If worktree data hasn't been loaded yet, triggers a background fetch
    /// and opens an empty sweep modal (data will arrive on next refresh).
    pub fn start_sweep(&mut self) {
        // Ensure worktree data is loaded (may not be if called from agents view)
        if self.worktrees.is_empty() {
            self.spawn_worktree_fetch();
        }

        let gone = git::get_gone_branches().unwrap_or_default();

        let mut candidates: Vec<SweepCandidate> = Vec::new();

        for wt in &self.worktrees {
            if wt.is_main {
                continue;
            }

            let status = self.git_statuses.get(&wt.path);
            let is_dirty = status.is_some_and(|s| s.is_dirty);
            let has_upstream = status.is_some_and(|s| s.has_upstream);

            // Determine reason: PR merged > PR closed > upstream gone > merged locally
            let reason = if let Some(ref pr) = wt.pr_info {
                match pr.state.as_str() {
                    "MERGED" => Some(SweepReason::PrMerged),
                    "CLOSED" => Some(SweepReason::PrClosed),
                    _ => {
                        if gone.contains(&wt.branch) {
                            Some(SweepReason::UpstreamGone)
                        } else {
                            None
                        }
                    }
                }
            } else if gone.contains(&wt.branch) {
                Some(SweepReason::UpstreamGone)
            } else if !has_upstream && !wt.has_unmerged {
                Some(SweepReason::MergedLocally)
            } else {
                None
            };

            let Some(reason) = reason else { continue };

            candidates.push(SweepCandidate {
                handle: wt.handle.clone(),
                path: wt.path.clone(),
                reason,
                is_dirty,
                selected: !is_dirty, // Pre-select non-dirty candidates
            });
        }

        self.pending_sweep = Some(SweepState {
            candidates,
            cursor: 0,
        });
    }

    /// Toggle selection of the current sweep candidate.
    pub fn sweep_toggle(&mut self) {
        if let Some(ref mut sweep) = self.pending_sweep
            && let Some(candidate) = sweep.candidates.get_mut(sweep.cursor)
            && !candidate.is_dirty
        {
            candidate.selected = !candidate.selected;
        }
    }

    /// Move cursor up in sweep modal.
    pub fn sweep_up(&mut self) {
        if let Some(ref mut sweep) = self.pending_sweep {
            sweep.cursor = sweep.cursor.saturating_sub(1);
        }
    }

    /// Move cursor down in sweep modal.
    pub fn sweep_down(&mut self) {
        if let Some(ref mut sweep) = self.pending_sweep
            && sweep.cursor + 1 < sweep.candidates.len()
        {
            sweep.cursor += 1;
        }
    }

    /// Execute sweep: remove all selected candidates.
    pub fn confirm_sweep(&mut self) {
        let Some(sweep) = self.pending_sweep.take() else {
            return;
        };

        let paths_to_remove: Vec<PathBuf> = sweep
            .candidates
            .iter()
            .filter(|c| c.selected)
            .map(|c| c.path.clone())
            .collect();

        for path in &paths_to_remove {
            self.do_remove_worktree(path, false);
        }
    }

    // ── Project picker methods ─────────────────────────────────────

    /// Discover projects from cached repo roots and open the picker modal.
    pub fn show_project_picker(&mut self) {
        // Deduplicate by project name, keeping one representative path per project
        let mut by_name: std::collections::BTreeMap<String, PathBuf> =
            std::collections::BTreeMap::new();

        for root in self.repo_roots.values() {
            let name = agent::extract_project_name(root);
            by_name.entry(name).or_insert_with(|| root.clone());
        }

        let projects: Vec<ProjectEntry> = by_name
            .into_iter()
            .map(|(name, path)| ProjectEntry { name, path })
            .collect();

        let current_name = self
            .worktree_project_override
            .as_ref()
            .map(|(name, _)| name.clone())
            .or_else(|| {
                self.current_worktree
                    .as_deref()
                    .map(agent::extract_project_name)
            });

        self.pending_project_picker = Some(ProjectPicker {
            projects,
            cursor: 0,
            filter: String::new(),
            current_name,
        });
    }

    /// Move cursor down in project picker.
    pub fn project_picker_down(&mut self) {
        if let Some(ref mut picker) = self.pending_project_picker {
            let filtered = picker.filtered();
            if !filtered.is_empty() && picker.cursor + 1 < filtered.len() {
                picker.cursor += 1;
            }
        }
    }

    /// Move cursor up in project picker.
    pub fn project_picker_up(&mut self) {
        if let Some(ref mut picker) = self.pending_project_picker {
            picker.cursor = picker.cursor.saturating_sub(1);
        }
    }

    /// Append a character to the project picker filter.
    pub fn project_picker_filter_append(&mut self, c: char) {
        if let Some(ref mut picker) = self.pending_project_picker {
            picker.filter.push(c);
            picker.cursor = 0;
        }
    }

    /// Delete the last character from the project picker filter.
    pub fn project_picker_filter_delete(&mut self) {
        if let Some(ref mut picker) = self.pending_project_picker {
            picker.filter.pop();
            picker.cursor = 0;
        }
    }

    /// Confirm project picker selection: set override and trigger refetch.
    pub fn confirm_project_picker(&mut self) {
        let Some(picker) = self.pending_project_picker.take() else {
            return;
        };
        let filtered = picker.filtered();
        let Some(&idx) = filtered.get(picker.cursor) else {
            return;
        };
        let selected = &picker.projects[idx];

        self.worktree_project_override = Some((selected.name.clone(), selected.path.clone()));
        self.worktrees.clear();
        self.all_worktrees.clear();
        self.last_worktree_fetch = std::time::Instant::now();
        self.spawn_worktree_fetch();

        // Switch to worktrees tab to show the result
        if self.active_tab != DashboardTab::Worktrees {
            self.active_tab = DashboardTab::Worktrees;
        }
    }

    /// Open a tmux window/session for the selected worktree via workflow::open,
    /// then close the dashboard.
    pub fn open_selected_worktree(&mut self) {
        let Some(selected) = self.worktree_table_state.selected() else {
            return;
        };
        let Some(worktree) = self.worktrees.get(selected) else {
            return;
        };

        let handle = worktree.handle.clone();

        let Ok(ctx) = workflow::WorkflowContext::new(self.config.clone(), self.mux.clone(), None)
        else {
            return;
        };

        let options = workflow::types::SetupOptions::new(false, false, true);
        if workflow::open(&handle, &ctx, options, false, false, None).is_ok() {
            self.should_jump = true;
        }
    }

    /// Jump to the selected worktree's agent or mux window.
    /// Tries the agent pane first, then falls back to workflow::open
    /// which switches to an existing window/session or creates one.
    pub fn jump_to_selected_worktree(&mut self) {
        let Some(selected) = self.worktree_table_state.selected() else {
            return;
        };
        let Some(worktree) = self.worktrees.get(selected) else {
            return;
        };

        // Try agent pane first for direct pane targeting
        if let Some(agent) = self.all_agents.iter().find(|a| a.path == worktree.path) {
            let target = agent.pane_id.clone();
            self.switch_to_pane_and_track(&target);
            return;
        }

        // Fall back to workflow::open (switches to existing or creates new)
        self.open_selected_worktree();
    }

    /// Update the preview for the selected worktree (git log)
    fn update_worktree_preview(&mut self) {
        let current_path = self
            .worktree_table_state
            .selected()
            .and_then(|idx| self.worktrees.get(idx))
            .map(|w| w.path.clone());

        if current_path != self.worktree_preview_path {
            self.worktree_preview_path = current_path.clone();
            self.worktree_preview = None;

            if let Some(path) = current_path {
                let tx = self.event_tx.clone();
                std::thread::spawn(move || {
                    let output = std::process::Command::new("git")
                        .args(["log", "--format=%h\t%ar\t%s", "-n", "20"])
                        .current_dir(&path)
                        .output();
                    if let Ok(out) = output {
                        let log = String::from_utf8_lossy(&out.stdout).to_string();
                        let _ = tx.send(AppEvent::WorktreeLog(path, log));
                    }
                });
            }
        }
    }
}
