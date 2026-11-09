//! Background thread spawning for git status and PR status fetches.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::command::sidebar::SnapshotHandle;
use crate::git;

use super::super::agent;
use super::App;
use super::types::AppEvent;

/// How long a git status query waits for the daemon. It answers from memory,
/// so a reply that is this late is not coming and we compute locally instead.
const DAEMON_GIT_TIMEOUT: Duration = Duration::from_millis(500);

/// Was `status` computed against the base this dashboard's config asks for?
///
/// The daemon resolves `main_branch` per worktree from the project config
/// governing it, so it usually agrees. It can still differ: this dashboard's
/// config may be layered differently (a profile, a `--config`), and a cached
/// reply can predate a config change. A status against another base must not
/// be mixed with the locally computed ones — a row would otherwise flip
/// between the two as its path enters and leaves the daemon's tracked set. An
/// empty `base_branch` is a detached HEAD, where no base took part in the
/// numbers.
fn base_is_ours(status: &git::GitStatus, main_branch: Option<&str>) -> bool {
    match main_branch.filter(|s| !s.is_empty()) {
        None => true,
        Some(main) => status.base_branch.is_empty() || status.base_branch == main,
    }
}

/// Emit a git status for each path: the daemon's when it answers, computed
/// locally otherwise, so `muxix dashboard` still works with no daemon running.
///
/// `prioritise` asks the daemon to refresh the named paths early. The statuses
/// it returns carry their fingerprint, so they are usable as the next sweep's
/// `prev` (unlike the ones pushed in sidebar snapshots, which are stripped).
fn resolve_git_statuses(
    daemon: &SnapshotHandle,
    paths: Vec<(PathBuf, Option<git::GitStatus>)>,
    main_branch: Option<&str>,
    prioritise: bool,
    mut emit: impl FnMut(PathBuf, git::GitStatus),
) {
    let from_daemon = daemon
        .request_git_statuses(
            paths.iter().map(|(p, _)| p.clone()).collect(),
            prioritise,
            DAEMON_GIT_TIMEOUT,
        )
        .unwrap_or_default();

    for (path, prev) in paths {
        let status = match from_daemon.get(&path) {
            Some(status) if base_is_ours(status, main_branch) => status.clone(),
            _ => git::get_git_status_cached(&path, main_branch, prev.as_ref()),
        };
        emit(path, status);
    }
}

impl App {
    /// Spawn a background thread to fetch git status for all agent worktrees
    pub(super) fn spawn_git_status_fetch(&mut self) {
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
        let main_branch = self.config.main_branch.clone();
        // Include both agent paths and worktree paths so the worktree view gets git status too
        let mut paths: Vec<PathBuf> = self.all_agents.iter().map(|a| a.path.clone()).collect();
        for wt in &self.worktrees {
            if !paths.contains(&wt.path) {
                paths.push(wt.path.clone());
            }
        }
        // Pair each path with the status we already hold (from the on-disk
        // cache or an earlier sweep) so an unmoved clean worktree costs one
        // git process instead of 5-7.
        let paths: Vec<(PathBuf, Option<git::GitStatus>)> = paths
            .into_iter()
            .map(|p| {
                let prev = self.git_statuses.get(&p).cloned();
                (p, prev)
            })
            .collect();
        let daemon = self.daemon.clone();
        let prioritise = std::mem::take(&mut self.git_prioritise);

        std::thread::spawn(move || {
            // Reset flag when thread completes (even on panic)
            struct ResetFlag(Arc<AtomicBool>);
            impl Drop for ResetFlag {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = ResetFlag(is_fetching);

            resolve_git_statuses(
                &daemon,
                paths,
                main_branch.as_deref(),
                prioritise,
                |path, status| {
                    let _ = tx.send(AppEvent::GitStatus(path, status));
                },
            );
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
        // Use all_agents (not filtered agents) because apply_filters() runs after
        // this in refresh(), so self.agents may still be empty on first call.
        let mut repo_branches: HashMap<PathBuf, Vec<String>> = HashMap::new();
        for agent in &self.all_agents {
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
            // Return false so the caller doesn't reset the timer — no fetch
            // was actually started, and we want to retry once git statuses
            // or worktree data arrive.
            return false;
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

#[cfg(test)]
mod git_status_source_tests {
    use super::*;
    use crate::cmd::take_git_spawns;
    use crate::command::sidebar::{connect_daemon, daemon_testing::fake_daemon};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::mpsc;

    /// The one status the fake daemon serves, for `path`.
    fn served(path: &std::path::Path) -> HashMap<PathBuf, git::GitStatus> {
        HashMap::from([(
            path.to_path_buf(),
            git::GitStatus {
                lines_added: 7,
                head_oid: Some("head-oid".to_string()),
                ..Default::default()
            },
        )])
    }

    /// Statuses for one path, as the background thread would collect them.
    fn resolve_one(daemon: &SnapshotHandle, path: &std::path::Path) -> git::GitStatus {
        resolve_one_against(daemon, path, None)
    }

    /// As [`resolve_one`], for a dashboard with `main_branch` configured.
    fn resolve_one_against(
        daemon: &SnapshotHandle,
        path: &std::path::Path,
        main_branch: Option<&str>,
    ) -> git::GitStatus {
        let mut got = Vec::new();
        resolve_git_statuses(
            daemon,
            vec![(path.to_path_buf(), None)],
            main_branch,
            true,
            |_, status| got.push(status),
        );
        assert_eq!(got.len(), 1, "one status per requested path");
        got.pop().unwrap()
    }

    /// Retry until the background connect is up and the daemon answers.
    fn wait_for_daemon(daemon: &SnapshotHandle, path: &std::path::Path, main_branch: Option<&str>) {
        for _ in 0..100 {
            if resolve_one_against(daemon, path, main_branch).lines_added == 7 {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the fake daemon never answered");
    }

    #[test]
    fn a_daemon_answer_costs_the_dashboard_no_git_process() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        let wt = dir.path().to_path_buf();
        fake_daemon(UnixListener::bind(&sock).unwrap(), usize::MAX, served(&wt));

        let (wake_tx, _wake_rx) = mpsc::sync_channel(1);
        let daemon = connect_daemon(&sock, wake_tx);
        wait_for_daemon(&daemon, &wt, None);

        take_git_spawns();
        let status = resolve_one(&daemon, &wt);
        assert_eq!(status.lines_added, 7, "the daemon's status was used");
        assert_eq!(
            status.head_oid.as_deref(),
            Some("head-oid"),
            "the fingerprint survives, so it can serve as the next prev"
        );
        assert_eq!(
            take_git_spawns(),
            0,
            "no git process for a path the daemon answered for"
        );
    }

    #[test]
    fn a_reply_computed_against_another_base_is_not_used() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        let wt = dir.path().to_path_buf();
        let mut statuses = served(&wt);
        // A reply that predates this dashboard's config change.
        statuses.get_mut(&wt).unwrap().base_branch = "master".to_string();
        fake_daemon(UnixListener::bind(&sock).unwrap(), usize::MAX, statuses);

        let (wake_tx, _wake_rx) = mpsc::sync_channel(1);
        let daemon = connect_daemon(&sock, wake_tx);
        wait_for_daemon(&daemon, &wt, Some("master"));

        take_git_spawns();
        let status = resolve_one_against(&daemon, &wt, Some("develop"));
        assert_ne!(
            status.lines_added, 7,
            "a status against another base must not be shown next to local ones"
        );
        assert!(
            take_git_spawns() > 0,
            "the dashboard recomputes it against its own base"
        );
    }

    #[test]
    fn without_a_daemon_the_dashboard_computes_statuses_itself() {
        let dir = tempfile::tempdir().unwrap();
        let (wake_tx, _wake_rx) = mpsc::sync_channel(1);
        let daemon = connect_daemon(&dir.path().join("absent.sock"), wake_tx);

        take_git_spawns();
        let status = resolve_one(&daemon, dir.path());
        assert_ne!(status.lines_added, 7, "nothing served this status");
        assert!(
            take_git_spawns() > 0,
            "with no daemon the dashboard runs git itself"
        );
    }

    #[test]
    fn a_daemon_that_dies_mid_session_falls_back_to_local_computation() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        let wt = dir.path().to_path_buf();
        // One answer, then the fake daemon closes; removing the socket leaves
        // nothing to reconnect to.
        fake_daemon(UnixListener::bind(&sock).unwrap(), 1, served(&wt));

        let (wake_tx, _wake_rx) = mpsc::sync_channel(1);
        let daemon = connect_daemon(&sock, wake_tx);
        wait_for_daemon(&daemon, &wt, None);
        std::fs::remove_file(&sock).unwrap();
        // The reader only notices the hangup once it reads, so wait for the
        // connection to actually be gone.
        for _ in 0..200 {
            if UnixStream::connect(&sock).is_err() && resolve_one(&daemon, &wt).lines_added != 7 {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        take_git_spawns();
        let status = resolve_one(&daemon, &wt);
        assert_ne!(status.lines_added, 7, "the dead daemon served nothing");
        assert!(
            take_git_spawns() > 0,
            "the dashboard keeps working by running git itself"
        );
    }
}
