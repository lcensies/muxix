//! Background thread spawning for git status and PR status fetches.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::git;

use super::super::agent;
use super::App;
use super::types::AppEvent;

impl App {
    /// Spawn a background thread to fetch git status for all agent worktrees
    pub(super) fn spawn_git_status_fetch(&self) {
        // Skip if a fetch is already in progress (prevents thread pile-up)
        if self
            .is_git_fetching
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }

        let tx = self.event_tx.clone();
        let is_fetching = self.is_git_fetching.clone();
        // Include both agent paths and worktree paths so the worktree view gets git status too
        let mut paths: Vec<PathBuf> = self.all_agents.iter().map(|a| a.path.clone()).collect();
        for wt in &self.worktrees {
            if !paths.contains(&wt.path) {
                paths.push(wt.path.clone());
            }
        }

        std::thread::spawn(move || {
            // Reset flag when thread completes (even on panic)
            struct ResetFlag(Arc<AtomicBool>);
            impl Drop for ResetFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = ResetFlag(is_fetching);

            for path in paths {
                let status = git::get_git_status(&path);
                let _ = tx.send(AppEvent::GitStatus(path, status));
            }
        });
    }

    /// Spawn a background thread to fetch PR status for all repos.
    /// Returns true if a fetch was started, false if one is already in progress.
    pub(super) fn spawn_pr_status_fetch(&self) -> bool {
        // Skip if already fetching
        if self
            .is_pr_fetching
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }

        // Collect branches per repo root from agents
        let mut repo_branches: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for agent in &self.agents {
            let Some(status) = self.git_statuses.get(&agent.path) else {
                continue;
            };
            let Some(ref branch) = status.branch else {
                continue;
            };
            if branch == "main" || branch == "master" {
                continue;
            }
            if let Some(repo_root) = self.repo_roots.get(&agent.path) {
                repo_branches
                    .entry(repo_root.clone())
                    .or_default()
                    .push(branch.clone());
            }
        }

        // Also collect branches from worktrees (keyed by main worktree path as repo root)
        // Group non-main worktrees by their project's main worktree path
        let main_paths: HashMap<String, PathBuf> = self
            .all_worktrees
            .iter()
            .filter(|w| w.is_main)
            .map(|w| {
                let project = agent::extract_project_name(&w.path);
                (project, w.path.clone())
            })
            .collect();
        for wt in &self.all_worktrees {
            if wt.is_main || wt.branch == "main" || wt.branch == "master" {
                continue;
            }
            let project = agent::extract_project_name(&wt.path);
            if let Some(repo_root) = main_paths.get(&project) {
                repo_branches
                    .entry(repo_root.clone())
                    .or_default()
                    .push(wt.branch.clone());
            }
        }

        // Deduplicate branches per repo
        for branches in repo_branches.values_mut() {
            branches.sort();
            branches.dedup();
        }

        if repo_branches.is_empty() {
            self.is_pr_fetching.store(false, Ordering::SeqCst);
            return true;
        }

        let tx = self.event_tx.clone();
        let is_fetching = self.is_pr_fetching.clone();

        // Identify the priority repo (current project) so it fetches first
        let priority_repo = self
            .worktree_project_override
            .as_ref()
            .map(|(_, p)| p.clone())
            .or_else(|| {
                self.current_worktree
                    .as_ref()
                    .and_then(|p| self.repo_roots.get(p).cloned())
            });

        std::thread::spawn(move || {
            struct ResetFlag(Arc<AtomicBool>);
            impl Drop for ResetFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = ResetFlag(is_fetching);

            // Sort repos so the priority repo (current project) is fetched first
            let mut repos: VecDeque<_> = repo_branches.into_iter().collect();
            if let Some(ref priority) = priority_repo {
                repos
                    .make_contiguous()
                    .sort_by_key(|(repo, _)| repo != priority);
            }

            // Fetch repos in parallel with bounded concurrency
            let queue = Arc::new(Mutex::new(repos));
            let workers = queue.lock().unwrap().len().min(4);

            std::thread::scope(|s| {
                for _ in 0..workers {
                    let queue = Arc::clone(&queue);
                    let tx = tx.clone();
                    s.spawn(move || {
                        loop {
                            let Some((repo_root, branches)) = queue.lock().unwrap().pop_front()
                            else {
                                break;
                            };
                            match crate::github::list_prs_for_branches(&repo_root, &branches) {
                                Ok(prs) => {
                                    let _ = tx.send(AppEvent::PrStatus(repo_root, prs));
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        "Failed to fetch PRs for {:?}: {}",
                                        repo_root,
                                        e
                                    );
                                }
                            }
                        }
                    });
                }
            });
        });

        true
    }
}
