use std::collections::HashSet;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

use crate::git;
use crate::multiplexer::{AgentStatus, create_backend, detect_backend};
use crate::state::{Completion, CompletionKind, StateStore};
use crate::util;
use crate::workflow;

/// Resolve a worktree name to its path, trying local git first then global agents.
///
/// Local resolution is preferred because it works even before an agent starts
/// (the worktree directory exists from `muxix add`). Global resolution requires
/// a running agent.
fn resolve_worktree_path(
    name: &str,
    mux: &dyn crate::multiplexer::Multiplexer,
) -> Result<std::path::PathBuf> {
    // Try local git resolution first (supports waiting for unstarted agents)
    if git::is_git_repo().unwrap_or(false) {
        match git::find_worktree(name) {
            Ok((path, _branch)) => return Ok(path),
            Err(e) if e.downcast_ref::<git::WorktreeNotFound>().is_some() => {}
            Err(e) => return Err(e),
        }
    }

    // Fall back to global agent resolution
    let (path, _agents) = workflow::resolve_worktree_agents(name, mux)?;
    Ok(path)
}

/// A status/outcome `wait --status` can be told to wait for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum WaitTarget {
    Working,
    Waiting,
    Done,
    Completed,
    Failed,
    Merged,
}

/// Canonical order used both to parse and to pick a deterministic hit when
/// several requested targets could match at once (e.g. `completed,merged`
/// once the worktree has been merged).
const ALL_TARGETS: [WaitTarget; 6] = [
    WaitTarget::Working,
    WaitTarget::Waiting,
    WaitTarget::Done,
    WaitTarget::Completed,
    WaitTarget::Failed,
    WaitTarget::Merged,
];

impl WaitTarget {
    fn label(self) -> &'static str {
        match self {
            WaitTarget::Working => "working",
            WaitTarget::Waiting => "waiting",
            WaitTarget::Done => "done",
            WaitTarget::Completed => "completed",
            WaitTarget::Failed => "failed",
            WaitTarget::Merged => "merged",
        }
    }
}

/// Parse a comma-separated `--status` value into the set of targets to wait for.
fn parse_targets(s: &str) -> Result<HashSet<WaitTarget>> {
    s.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| match part {
            "working" => Ok(WaitTarget::Working),
            "waiting" => Ok(WaitTarget::Waiting),
            "done" => Ok(WaitTarget::Done),
            "completed" => Ok(WaitTarget::Completed),
            "failed" => Ok(WaitTarget::Failed),
            "merged" => Ok(WaitTarget::Merged),
            other => Err(anyhow!(
                "Invalid status '{}'. Must be one of: working, waiting, done, completed, failed, merged",
                other
            )),
        })
        .collect()
}

/// Pure matcher: does this single observation reach `target`?
///
/// `Working`/`Waiting`/`Done` match the turn-boundary `AgentStatus`.
/// `Completed`/`Failed` match the agent-authored `CompletionKind`; `Completed`
/// also folds in the historical "worktree disappeared" success fallback
/// (`seen && !wt_exists`) so a merge still satisfies a plain `--status completed`
/// wait. `Failed` has no such fallback: a vanished worktree is not a failure.
/// `Merged` is that same disappearance condition, requested explicitly.
fn target_hit(
    target: WaitTarget,
    status: Option<AgentStatus>,
    completion_kind: Option<CompletionKind>,
    wt_exists: bool,
    seen: bool,
) -> bool {
    match target {
        WaitTarget::Working => status == Some(AgentStatus::Working),
        WaitTarget::Waiting => status == Some(AgentStatus::Waiting),
        WaitTarget::Done => status == Some(AgentStatus::Done),
        WaitTarget::Failed => completion_kind == Some(CompletionKind::Failed),
        WaitTarget::Completed => {
            completion_kind == Some(CompletionKind::Completed) || (seen && !wt_exists)
        }
        WaitTarget::Merged => seen && !wt_exists,
    }
}

/// Scan `observations` (one per live agent pane, or a single `(None, None)`
/// placeholder when the agent has disappeared) against the requested
/// `targets` in canonical order, returning the first target hit and the
/// completion data behind it (for feedback text), if any.
fn find_target_hit(
    targets: &HashSet<WaitTarget>,
    observations: &[(Option<AgentStatus>, Option<Completion>)],
    wt_exists: bool,
    seen: bool,
) -> Option<(WaitTarget, Option<Completion>)> {
    for &t in ALL_TARGETS.iter().filter(|t| targets.contains(t)) {
        for (status, completion) in observations {
            let kind = completion.as_ref().map(|c| c.kind);
            if target_hit(t, *status, kind, wt_exists, seen) {
                return Some((t, completion.clone()));
            }
        }
    }
    None
}

/// Print the line for a worktree that reached `target`, including feedback
/// text on `completed`/`failed` when the agent provided any.
fn print_reached(name: &str, target: WaitTarget, completion: Option<&Completion>, elapsed: &str) {
    match target {
        WaitTarget::Completed | WaitTarget::Failed => {
            match completion.and_then(|c| c.feedback.as_deref()).filter(|fb| !fb.is_empty()) {
                Some(feedback) => {
                    eprintln!("{}: {} ({}) — {}", name, target.label(), elapsed, feedback)
                }
                None => eprintln!("{}: {} ({})", name, target.label(), elapsed),
            }
        }
        _ => eprintln!("{}: {} ({})", name, target.label(), elapsed),
    }
}

pub fn run(
    worktree_names: &[String],
    target_status: &str,
    timeout_secs: Option<u64>,
    any: bool,
) -> Result<()> {
    let targets = parse_targets(target_status)?;
    let mux = create_backend(detect_backend());
    let start = Instant::now();

    // Resolve worktree paths upfront (local git first, then global agents)
    let worktree_paths: Vec<_> = worktree_names
        .iter()
        .map(|name| {
            let path = resolve_worktree_path(name, mux.as_ref())?;
            Ok((name.clone(), path))
        })
        .collect::<Result<Vec<_>>>()?;

    let mut reached: HashSet<String> = HashSet::new();
    let mut exited: HashSet<String> = HashSet::new();
    let mut seen_agent: HashSet<String> = HashSet::new();

    loop {
        // Check timeout
        if let Some(timeout) = timeout_secs
            && start.elapsed() > Duration::from_secs(timeout)
        {
            let remaining: Vec<_> = worktree_names
                .iter()
                .filter(|n| !reached.contains(n.as_str()) && !exited.contains(n.as_str()))
                .collect();
            if !remaining.is_empty() {
                eprintln!(
                    "Timeout waiting for: {}",
                    remaining
                        .iter()
                        .map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            if !exited.is_empty() {
                let mut names: Vec<_> = exited.iter().cloned().collect();
                names.sort();
                eprintln!("Also exited unexpectedly: {}", names.join(", "));
            }
            std::process::exit(1);
        }

        // Load current agent state
        let agent_panes =
            StateStore::new().and_then(|store| store.load_reconciled_agents(mux.as_ref()))?;
        let completions = crate::state::completion_by_pane(mux.as_ref());

        for (name, wt_path) in &worktree_paths {
            if reached.contains(name) || exited.contains(name) {
                continue;
            }

            let matching = workflow::match_agents_to_worktree(&agent_panes, wt_path);

            if !matching.is_empty() {
                seen_agent.insert(name.clone());

                let observations: Vec<_> = matching
                    .iter()
                    .map(|a| (a.status, completions.get(&a.pane_id).cloned()))
                    .collect();

                if let Some((t, completion)) =
                    find_target_hit(&targets, &observations, true, true)
                {
                    let elapsed = util::format_elapsed_duration(start.elapsed());
                    print_reached(name, t, completion.as_ref(), &elapsed);
                    reached.insert(name.clone());

                    if any {
                        return Ok(());
                    }
                }
            } else if seen_agent.contains(name) {
                // Agent was previously running but has disappeared.
                let wt_exists = wt_path.exists();

                if let Some((t, completion)) =
                    find_target_hit(&targets, &[(None, None)], wt_exists, true)
                {
                    // Either `merged` was requested, or `completed` was and the
                    // worktree is gone — the historical merge-success fallback.
                    let elapsed = util::format_elapsed_duration(start.elapsed());
                    print_reached(name, t, completion.as_ref(), &elapsed);
                    reached.insert(name.clone());

                    if any {
                        return Ok(());
                    }
                } else if wt_exists {
                    // Worktree still exists but the agent is gone and none of
                    // the requested targets treat that as a hit: crashed/exited.
                    eprintln!("{}: agent exited unexpectedly", name);
                    exited.insert(name.clone());
                }
                // else: worktree is gone but none of the requested targets
                // (e.g. `working`/`failed` only) treat that as reached; nothing
                // more will happen for this name, so fall through to timeout.
            }
            // If we haven't seen an agent yet, still wait -- the agent may not
            // have started yet. The timeout flag handles the overall deadline.
        }

        // Loop ends once every worktree has either reached its target or exited.
        if reached.len() + exited.len() == worktree_paths.len() {
            if !exited.is_empty() {
                std::process::exit(3);
            }
            return Ok(());
        }

        thread::sleep(Duration::from_secs(2));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(values: &[WaitTarget]) -> HashSet<WaitTarget> {
        values.iter().copied().collect()
    }

    // -- parse_targets --

    #[test]
    fn parse_targets_single_value() {
        assert_eq!(parse_targets("done").unwrap(), targets(&[WaitTarget::Done]));
    }

    #[test]
    fn parse_targets_comma_list() {
        assert_eq!(
            parse_targets("completed,failed").unwrap(),
            targets(&[WaitTarget::Completed, WaitTarget::Failed])
        );
    }

    #[test]
    fn parse_targets_trims_whitespace() {
        assert_eq!(
            parse_targets(" working , merged ").unwrap(),
            targets(&[WaitTarget::Working, WaitTarget::Merged])
        );
    }

    #[test]
    fn parse_targets_invalid_lists_all_six() {
        let err = parse_targets("bogus").unwrap_err().to_string();
        for name in ["working", "waiting", "done", "completed", "failed", "merged"] {
            assert!(err.contains(name), "error should mention '{name}': {err}");
        }
    }

    // -- target_hit --

    #[test]
    fn target_hit_status_targets_match_agent_status() {
        assert!(target_hit(
            WaitTarget::Working,
            Some(AgentStatus::Working),
            None,
            true,
            true
        ));
        assert!(!target_hit(
            WaitTarget::Working,
            Some(AgentStatus::Done),
            None,
            true,
            true
        ));
        assert!(target_hit(
            WaitTarget::Done,
            Some(AgentStatus::Done),
            None,
            true,
            true
        ));
    }

    #[test]
    fn target_hit_completed_matches_completion_kind() {
        assert!(target_hit(
            WaitTarget::Completed,
            None,
            Some(CompletionKind::Completed),
            true,
            true
        ));
        assert!(!target_hit(
            WaitTarget::Completed,
            None,
            Some(CompletionKind::Failed),
            true,
            true
        ));
    }

    #[test]
    fn target_hit_failed_matches_completion_kind_only_no_merge_fallback() {
        assert!(target_hit(
            WaitTarget::Failed,
            None,
            Some(CompletionKind::Failed),
            true,
            true
        ));
        // A vanished worktree is not a failure, unlike the `completed` fallback.
        assert!(!target_hit(WaitTarget::Failed, None, None, false, true));
    }

    #[test]
    fn target_hit_completed_falls_back_to_merge_success() {
        assert!(target_hit(WaitTarget::Completed, None, None, false, true));
        // Never seen: not a fallback hit (agent never actually ran here).
        assert!(!target_hit(WaitTarget::Completed, None, None, false, false));
        // Worktree still present: not merged, no fallback.
        assert!(!target_hit(WaitTarget::Completed, None, None, true, true));
    }

    #[test]
    fn target_hit_merged_requires_seen_and_gone() {
        assert!(target_hit(WaitTarget::Merged, None, None, false, true));
        assert!(!target_hit(WaitTarget::Merged, None, None, true, true));
        assert!(!target_hit(WaitTarget::Merged, None, None, false, false));
    }

    // -- find_target_hit --

    #[test]
    fn find_target_hit_picks_first_matching_target_in_canonical_order() {
        let targets = targets(&[WaitTarget::Merged, WaitTarget::Completed]);
        // Agent gone, worktree gone: both `completed` (fallback) and `merged`
        // would match; canonical order prefers `completed`.
        let hit = find_target_hit(&targets, &[(None, None)], false, true);
        assert_eq!(hit.map(|(t, _)| t), Some(WaitTarget::Completed));
    }

    #[test]
    fn find_target_hit_none_when_nothing_matches() {
        let targets = targets(&[WaitTarget::Working]);
        let hit = find_target_hit(
            &targets,
            &[(Some(AgentStatus::Done), None)],
            true,
            true,
        );
        assert!(hit.is_none());
    }
}
