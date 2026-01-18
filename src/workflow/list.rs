use anyhow::{Result, anyhow};
use std::collections::HashSet;

use crate::multiplexer::{Multiplexer, util};
use crate::{config, git, github, spinner};

use super::types::WorktreeInfo;

/// List all worktrees with their status
pub fn list(
    config: &config::Config,
    mux: &dyn Multiplexer,
    fetch_pr_status: bool,
) -> Result<Vec<WorktreeInfo>> {
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    let worktrees_data = git::list_worktrees()?;

    if worktrees_data.is_empty() {
        return Ok(Vec::new());
    }

    // Check mux status and get all windows once to avoid repeated process calls
    let mux_windows: HashSet<String> = if mux.is_running().unwrap_or(false) {
        mux.get_all_window_names().unwrap_or_default()
    } else {
        HashSet::new()
    };

    // Get the main branch for unmerged checks
    let main_branch = git::get_default_branch().ok();

    // Get all unmerged branches in one go for efficiency
    // Prefer checking against remote tracking branch for more accurate results
    let unmerged_branches = main_branch
        .as_deref()
        .and_then(|main| git::get_merge_base(main).ok())
        .and_then(|base| git::get_unmerged_branches(&base).ok())
        .unwrap_or_default(); // Use an empty set on failure

    // Batch fetch all PRs if requested (single API call)
    let pr_map = if fetch_pr_status {
        spinner::with_spinner("Fetching PR status", || {
            Ok(github::list_prs().unwrap_or_default())
        })?
    } else {
        std::collections::HashMap::new()
    };

    let prefix = config.window_prefix();
    let worktrees: Vec<WorktreeInfo> = worktrees_data
        .into_iter()
        .map(|(path, branch)| {
            // Extract handle from worktree path basename (the source of truth)
            let handle = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&branch)
                .to_string();

            // Use handle for mux window check, not branch name
            let prefixed_window_name = util::prefixed(prefix, &handle);
            let has_mux_window = mux_windows.contains(&prefixed_window_name);

            // Check for unmerged commits, but only if this isn't the main branch
            let has_unmerged = if let Some(ref main) = main_branch {
                if branch == *main || branch == "(detached)" {
                    false
                } else {
                    unmerged_branches.contains(&branch)
                }
            } else {
                false
            };

            // Lookup PR info from batch fetch
            let pr_info = pr_map.get(&branch).cloned();

            WorktreeInfo {
                branch,
                path,
                has_mux_window,
                has_unmerged,
                pr_info,
            }
        })
        .collect();

    Ok(worktrees)
}
