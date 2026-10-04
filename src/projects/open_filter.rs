//! Which worktrees `muxix start` / `muxix project open` restore.
//!
//! The decision needs to know whether an agent ever ran in a worktree, so this
//! module turns the flat agent state store into a handle → states lookup.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::cmd::Cmd;
use crate::config::ProjectOpenFilter;
use crate::multiplexer::AgentStatus;
use crate::state::{AgentState, StateStore};
use crate::util::canon_or_self;

/// Group persisted agent states by the worktree handle they ran in.
///
/// `backend` / `instance` are the live multiplexer's identity
/// (`Multiplexer::name` / `Multiplexer::instance_id`); states from another
/// backend or another server instance are dropped.
///
/// Matching uses the same rule as `workflow::resurrect::plan`: an agent belongs
/// to a worktree when its canonical `workdir` equals the canonical worktree root
/// or is a descendant of it (agents may run in a subdirectory). States that
/// match no given worktree (other project, removed worktree) are dropped.
pub fn states_by_handle<'a, I>(
    store: &StateStore,
    backend: &str,
    instance: &str,
    worktree_paths: I,
) -> Result<HashMap<String, Vec<AgentState>>>
where
    I: IntoIterator<Item = &'a Path>,
{
    let worktrees: Vec<(PathBuf, String)> = worktree_paths
        .into_iter()
        .filter_map(|p| {
            let handle = p.file_name()?.to_string_lossy().to_string();
            Some((canon_or_self(p), handle))
        })
        .collect();

    let mut by_handle: HashMap<String, Vec<AgentState>> = HashMap::new();
    for agent in store.list_all_agents()? {
        if agent.pane_key.backend != backend || agent.pane_key.instance != instance {
            continue;
        }
        let workdir = canon_or_self(&agent.workdir);
        if let Some((_, handle)) = worktrees
            .iter()
            .find(|(root, _)| workdir == *root || workdir.starts_with(root))
        {
            by_handle.entry(handle.clone()).or_default().push(agent);
        }
    }
    Ok(by_handle)
}

/// Should a worktree with these agent states be opened?
///
/// Pure: all filesystem and git facts arrive as `newest_repo_ts` (newest of the
/// worktree's last commit time and git index mtime) and `now`, both seconds
/// since the epoch.
pub fn keep(
    filter: ProjectOpenFilter,
    days: u64,
    states: &[AgentState],
    newest_repo_ts: Option<u64>,
    now: u64,
) -> bool {
    match filter {
        ProjectOpenFilter::All => true,
        ProjectOpenFilter::Active => !states.is_empty(),
        ProjectOpenFilter::Unfinished => states
            .iter()
            .any(|s| s.completion.is_none() && s.status != Some(AgentStatus::Done)),
        ProjectOpenFilter::Recent => {
            let newest = states
                .iter()
                .flat_map(|s| [Some(s.updated_ts), s.status_ts])
                .chain([newest_repo_ts])
                .flatten()
                .max();
            // A timestamp in the future (clock skew, a touched index) counts as now.
            newest.is_some_and(|ts| now.saturating_sub(ts) <= days * 86_400)
        }
    }
}

/// The worktree's own recency signal for `recent`: newest of its last commit
/// time and its git index mtime, in seconds since the epoch.
///
/// `None` when neither is readable (fresh repo with no commits and no index, or
/// not a git worktree at all). Agent-state timestamps are folded in by `keep`,
/// so this returns the git signal only.
///
/// ponytail: commit time plus index mtime is a proxy for "was this worktree
/// edited recently" — it misses edits that were never staged or committed
/// (ceiling: a worktree whose files changed but whose index did not looks
/// stale). Upgrade path is an mtime walk of the worktree, which costs a full
/// tree scan per worktree on every `muxix start`; not worth it.
pub fn newest_repo_ts(worktree: &Path) -> Option<u64> {
    let git = |args: &[&str]| {
        Cmd::new("git")
            .args(args)
            .workdir(worktree)
            .run_and_capture_stdout()
            .ok()
    };

    let commit_ts = git(&["log", "-1", "--format=%ct"]).and_then(|s| s.parse::<u64>().ok());

    // `--git-path` resolves the index for linked worktrees too (it lives under
    // the main repo's .git/worktrees/<name>/). It may answer relative to the
    // worktree root; join is a no-op for the absolute case.
    let index_ts = git(&["rev-parse", "--git-path", "index"])
        .and_then(|p| std::fs::metadata(worktree.join(p)).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    commit_ts.max(index_ts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Completion, CompletionKind, PaneKey};
    use tempfile::TempDir;

    /// An agent that is mid-work: no completion claim, status `working`.
    fn unfinished(pane_id: &str, instance: &str, workdir: &Path) -> AgentState {
        AgentState {
            agent_id: format!("agent-{pane_id}"),
            pane_key: PaneKey {
                backend: "tmux".to_string(),
                instance: instance.to_string(),
                pane_id: pane_id.to_string(),
            },
            workdir: workdir.to_path_buf(),
            status: Some(AgentStatus::Working),
            status_ts: Some(100),
            pane_title: None,
            pane_pid: 1,
            command: "node".to_string(),
            updated_ts: 100,
            window_name: None,
            session_name: None,
            boot_id: None,
            agent_kind: None,
            sandbox_id: None,
            checkpoint_path: None,
            checkpoint_ts: None,
            pipeline_node_id: None,
            pipeline_node_title: None,
            activity: None,
            runtime: None,
            completion: None,
        }
    }

    fn seed(store: &StateStore, pane_id: &str, instance: &str, workdir: &Path) {
        store
            .upsert_agent(&unfinished(pane_id, instance, workdir))
            .unwrap();
    }

    const NOW: u64 = 1_000_000_000;
    const WEEK: u64 = 7 * 86_400;

    fn one(mutate: impl FnOnce(&mut AgentState)) -> Vec<AgentState> {
        let mut state = unfinished("%1", "default", Path::new("/wt"));
        mutate(&mut state);
        vec![state]
    }

    #[test]
    fn all_keeps_everything_even_a_never_agent_worktree() {
        for (states, repo_ts) in [
            (vec![], None),
            (vec![], Some(NOW - 10 * WEEK)),
            (one(|_| {}), None),
        ] {
            assert!(keep(ProjectOpenFilter::All, 7, &states, repo_ts, NOW));
        }
    }

    #[test]
    fn active_keeps_any_worktree_an_agent_ran_in() {
        // Any state at all is enough, finished or not.
        assert!(keep(ProjectOpenFilter::Active, 7, &one(|_| {}), None, NOW));
        assert!(keep(
            ProjectOpenFilter::Active,
            7,
            &one(|s| {
                s.status = Some(AgentStatus::Done);
                s.completion = Some(Completion {
                    kind: CompletionKind::Completed,
                    feedback: None,
                    ts: NOW,
                });
            }),
            None,
            NOW,
        ));
        // A worktree no agent ever ran in is dropped, even if freshly edited.
        assert!(!keep(ProjectOpenFilter::Active, 7, &[], Some(NOW), NOW));
    }

    #[test]
    fn unfinished_separates_a_completion_claim_from_mid_work() {
        // The pair differs only in `completion`.
        assert!(keep(
            ProjectOpenFilter::Unfinished,
            7,
            &one(|_| {}),
            None,
            NOW
        ));
        for kind in [CompletionKind::Completed, CompletionKind::Failed] {
            assert!(
                !keep(
                    ProjectOpenFilter::Unfinished,
                    7,
                    &one(|s| s.completion = Some(Completion {
                        kind,
                        feedback: None,
                        ts: NOW,
                    })),
                    None,
                    NOW,
                ),
                "{kind:?} is a finished claim"
            );
        }
        // `done` status alone (no completion claim) also counts as finished.
        assert!(!keep(
            ProjectOpenFilter::Unfinished,
            7,
            &one(|s| s.status = Some(AgentStatus::Done)),
            None,
            NOW,
        ));
        // One unfinished agent among finished ones keeps the worktree.
        let mut mixed = one(|s| s.status = Some(AgentStatus::Done));
        mixed.extend(one(|_| {}));
        assert!(keep(ProjectOpenFilter::Unfinished, 7, &mixed, None, NOW));
        // Never-agent worktree: nothing unfinished to resume.
        assert!(!keep(ProjectOpenFilter::Unfinished, 7, &[], Some(NOW), NOW));
    }

    #[test]
    fn recent_window_boundary_from_the_repo_signal() {
        let recent = |repo_ts: Option<u64>| keep(ProjectOpenFilter::Recent, 7, &[], repo_ts, NOW);
        assert!(recent(Some(NOW - WEEK + 1)), "just inside the window");
        assert!(recent(Some(NOW - WEEK)), "exactly on the window edge");
        assert!(!recent(Some(NOW - WEEK - 1)), "just outside the window");
        // No signal at all (never-agent worktree, no commits, no index).
        assert!(!recent(None));
        // Clock skew: a future timestamp counts as now, not as ancient.
        assert!(recent(Some(NOW + 5 * WEEK)));
    }

    #[test]
    fn recent_also_uses_agent_state_timestamps() {
        // An old repo but a fresh agent state keeps the worktree...
        assert!(keep(
            ProjectOpenFilter::Recent,
            7,
            &one(|s| {
                s.updated_ts = NOW - WEEK + 1;
                s.status_ts = None;
            }),
            Some(NOW - 10 * WEEK),
            NOW,
        ));
        // ...and status_ts counts too, even when updated_ts is stale.
        assert!(keep(
            ProjectOpenFilter::Recent,
            7,
            &one(|s| {
                s.updated_ts = NOW - 10 * WEEK;
                s.status_ts = Some(NOW - WEEK + 1);
            }),
            None,
            NOW,
        ));
        // All signals stale: dropped regardless of unfinished work.
        assert!(!keep(
            ProjectOpenFilter::Recent,
            7,
            &one(|s| {
                s.updated_ts = NOW - WEEK - 1;
                s.status_ts = Some(NOW - WEEK - 1);
            }),
            Some(NOW - WEEK - 1),
            NOW,
        ));
        // `days` is what moves the edge.
        assert!(keep(
            ProjectOpenFilter::Recent,
            30,
            &[],
            Some(NOW - WEEK - 1),
            NOW
        ));
    }

    #[test]
    fn groups_by_handle_and_filters_foreign_states() {
        let dir = TempDir::new().unwrap();
        let store = StateStore::with_path(dir.path().join("state")).unwrap();

        let wt_a = dir.path().join("wt-a");
        let wt_b = dir.path().join("wt-b");
        let other = dir.path().join("elsewhere");
        std::fs::create_dir_all(wt_a.join("sub")).unwrap();
        std::fs::create_dir_all(&wt_b).unwrap();
        std::fs::create_dir_all(&other).unwrap();

        seed(&store, "%1", "default", &wt_a);
        // descendant workdir still matches its worktree root
        seed(&store, "%2", "default", &wt_a.join("sub"));
        // another server instance: not ours
        seed(&store, "%3", "other-socket", &wt_b);
        // no worktree covers this one
        seed(&store, "%4", "default", &other);

        let paths = [wt_a.as_path(), wt_b.as_path()];
        let map = states_by_handle(&store, "tmux", "default", paths).unwrap();

        assert_eq!(map.len(), 1, "only wt-a has states for this instance");
        assert_eq!(map["wt-a"].len(), 2);
    }

    #[test]
    fn newest_repo_ts_reads_commit_and_index_but_not_plain_dirs() {
        let dir = TempDir::new().unwrap();
        let plain = dir.path().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(newest_repo_ts(&plain), None, "not a git worktree");

        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "knxQcTV@CkpQgnx.ru"]);
        git(&["config", "user.name", "Test User"]);
        std::fs::write(repo.join("a.txt"), "hi\n").unwrap();
        git(&["add", "a.txt"]);
        git(&["commit", "-m", "init"]);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let ts = newest_repo_ts(&repo).expect("committed repo has a recency signal");
        assert!(
            now.abs_diff(ts) < 300,
            "expected a just-now timestamp, got {ts} vs {now}"
        );
    }
}
