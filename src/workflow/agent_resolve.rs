use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::git;
use crate::multiplexer::{AgentPane, Multiplexer};
use crate::state::StateStore;
use crate::util::canon_or_self;

/// Parsed agent target selector.
enum AgentSelector {
    /// Plain name, resolved locally first then globally.
    Local(String),
    /// Qualified `project:handle` for cross-project targeting.
    Qualified { project: String, handle: String },
}

impl AgentSelector {
    fn parse(s: &str) -> Self {
        // Colon is invalid in git branch/ref names, so no collision with branch names
        if let Some((project, handle)) = s.split_once(':')
            && !project.is_empty()
            && !handle.is_empty()
        {
            return Self::Qualified {
                project: project.to_string(),
                handle: handle.to_string(),
            };
        }
        Self::Local(s.to_string())
    }
}

/// Walk up from `path` to find the containing worktree/repo root.
///
/// Git worktrees have a `.git` file; regular repos have a `.git` directory.
/// Returns `None` if no `.git` is found (e.g. path is outside any repo).
pub fn find_worktree_root(path: &Path) -> Option<PathBuf> {
    let mut current = path;
    loop {
        if current.join(".git").exists() {
            return Some(current.to_path_buf());
        }
        current = current.parent()?;
    }
}

/// Resolve a worktree name to its agent panes.
///
/// Resolution strategy:
/// 1. Parse the selector (`project:handle` or plain name)
/// 2. For plain names: try local git worktree first, fall back to global on
///    `WorktreeNotFound` or when not in a git repo
/// 3. For qualified names: go straight to global resolution
/// 4. Global resolution matches by worktree root directory name, with
///    disambiguation on ambiguity
///
/// Returns the worktree path and matching agent panes (may be empty if no agent is running).
pub fn resolve_worktree_agents(
    name: &str,
    mux: &dyn Multiplexer,
) -> Result<(PathBuf, Vec<AgentPane>)> {
    match AgentSelector::parse(name) {
        AgentSelector::Qualified { project, handle } => {
            let agent_panes =
                StateStore::new().and_then(|store| store.load_reconciled_agents(mux))?;
            resolve_global_agents(&agent_panes, &handle, Some(&project))
        }
        AgentSelector::Local(local_name) => {
            // Try local git resolution first
            let in_git_repo = git::is_git_repo().unwrap_or(false);
            let local_result = if in_git_repo {
                match git::find_worktree(&local_name) {
                    Ok((worktree_path, _branch)) => {
                        let agent_panes = StateStore::new()
                            .and_then(|store| store.load_reconciled_agents(mux))?;
                        Some(Ok(resolve_local_agents(agent_panes, &worktree_path)))
                    }
                    Err(e) if e.downcast_ref::<git::WorktreeNotFound>().is_some() => None,
                    Err(e) => Some(Err(e)),
                }
            } else {
                None
            };

            match local_result {
                Some(Ok(result)) => Ok(result),
                Some(Err(e)) => Err(e),
                None => {
                    // Fall back to global resolution
                    let agent_panes =
                        StateStore::new().and_then(|store| store.load_reconciled_agents(mux))?;
                    resolve_global_agents(&agent_panes, &local_name, None)
                }
            }
        }
    }
}

/// Match agents against a known local worktree path.
fn resolve_local_agents(
    agent_panes: Vec<AgentPane>,
    worktree_path: &Path,
) -> (PathBuf, Vec<AgentPane>) {
    let canon_wt_path = canon_or_self(worktree_path);
    let matching: Vec<AgentPane> = agent_panes
        .into_iter()
        .filter(|a| {
            let canon_agent_path = canon_or_self(&a.path);
            canon_agent_path == canon_wt_path || canon_agent_path.starts_with(&canon_wt_path)
        })
        .collect();
    (worktree_path.to_path_buf(), matching)
}

/// Search all reconciled agents globally by worktree directory name.
///
/// Groups agents by their worktree root. If `project` is provided, also filters
/// by the parent directory name. Returns an error on ambiguity with suggestions.
fn resolve_global_agents(
    agent_panes: &[AgentPane],
    handle: &str,
    project: Option<&str>,
) -> Result<(PathBuf, Vec<AgentPane>)> {
    // Group agents by their worktree root
    let mut by_root: HashMap<PathBuf, Vec<&AgentPane>> = HashMap::new();

    for agent in agent_panes {
        let wt_root = match find_worktree_root(&agent.path) {
            Some(root) => root,
            None => agent.path.clone(),
        };

        let root_name = wt_root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        if root_name != handle {
            continue;
        }

        if let Some(proj) = project {
            let parent_name = wt_root
                .parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            if parent_name != proj {
                continue;
            }
        }

        by_root.entry(wt_root).or_default().push(agent);
    }

    match by_root.len() {
        0 => Err(anyhow!(
            "No agent found matching '{}'",
            format_selector(handle, project)
        )),
        1 => {
            let (root, agents) = by_root.into_iter().next().unwrap();
            Ok((root, agents.into_iter().cloned().collect()))
        }
        _ => {
            let mut options: Vec<String> = by_root
                .keys()
                .filter_map(|root| {
                    let dir = root.file_name()?.to_str()?;
                    let parent = root.parent()?.file_name()?.to_str()?;
                    Some(format!("{}:{}", parent, dir))
                })
                .collect();
            options.sort();
            Err(anyhow!(
                "Ambiguous agent name '{}'. Found in multiple projects:\n  {}\n\nUse 'project:handle' to disambiguate.",
                handle,
                options.join("\n  ")
            ))
        }
    }
}

fn format_selector(handle: &str, project: Option<&str>) -> String {
    match project {
        Some(proj) => format!("{}:{}", proj, handle),
        None => handle.to_string(),
    }
}

/// Resolve a worktree name to exactly one agent pane (the first/primary).
///
/// Returns an error if no agent is running in the worktree.
pub fn resolve_worktree_agent(name: &str, mux: &dyn Multiplexer) -> Result<(PathBuf, AgentPane)> {
    let (path, agents) = resolve_worktree_agents(name, mux)?;
    let agent = agents
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No agent running in worktree '{}'", name))?;
    Ok((path, agent))
}

/// Match agents to a worktree path from a pre-loaded agent list.
///
/// Used by `status` and `wait` commands that load agents once and match
/// multiple worktrees, avoiding repeated calls to `load_reconciled_agents`.
pub fn match_agents_to_worktree<'a>(
    agents: &'a [AgentPane],
    worktree_path: &Path,
) -> Vec<&'a AgentPane> {
    let canon_wt = canon_or_self(worktree_path);
    agents
        .iter()
        .filter(|a| {
            let canon_agent = canon_or_self(&a.path);
            canon_agent == canon_wt || canon_agent.starts_with(&canon_wt)
        })
        .collect()
}
