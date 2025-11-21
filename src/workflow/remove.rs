use anyhow::{Context, Result, anyhow};

use crate::{config, git};
use tracing::{debug, info};

use super::cleanup;
use super::types::RemoveResult;

/// Remove a worktree without merging
pub fn remove(
    branch_name: &str,
    force: bool,
    delete_remote: bool,
    keep_branch: bool,
    config: &config::Config,
) -> Result<RemoveResult> {
    info!(
        branch = branch_name,
        force, delete_remote, keep_branch, "remove:start"
    );
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    // Get the main worktree root for safety checks
    let main_worktree_root = git::get_main_worktree_root()
        .context("Could not find main worktree to run remove operations")?;

    // Get worktree path - this also validates that the worktree exists
    let worktree_path = git::get_worktree_path(branch_name)
        .with_context(|| format!("No worktree found for branch '{}'", branch_name))?;
    debug!(branch = branch_name, path = %worktree_path.display(), "remove:worktree resolved");

    // Safety Check: Prevent deleting the main worktree itself, regardless of branch.
    let is_main_worktree = match (
        worktree_path.canonicalize(),
        main_worktree_root.canonicalize(),
    ) {
        (Ok(canon_wt_path), Ok(canon_main_path)) => {
            // Best case: both paths exist and can be resolved. This is the most reliable check.
            canon_wt_path == canon_main_path
        }
        _ => {
            // Fallback: If canonicalization fails on either path (e.g., directory was
            // manually removed, broken symlink), compare the raw paths provided by git.
            // This is a critical safety net.
            worktree_path == main_worktree_root
        }
    };

    if is_main_worktree {
        return Err(anyhow!(
            "Cannot remove branch '{}' because it is checked out in the main worktree at '{}'. \
            Switch the main worktree to a different branch first, or create a linked worktree for '{}'.",
            branch_name,
            main_worktree_root.display(),
            branch_name
        ));
    }

    // Safety Check: Prevent deleting the main branch by name (secondary check)
    let main_branch = git::get_default_branch()
        .context("Failed to determine the main branch. You can specify it in .workmux.yaml")?;
    if branch_name == main_branch {
        return Err(anyhow!("Cannot delete the main branch ('{}')", main_branch));
    }

    if worktree_path.exists() && git::has_uncommitted_changes(&worktree_path)? && !force {
        return Err(anyhow!(
            "Worktree has uncommitted changes. Use --force to delete anyway."
        ));
    }

    // Note: Unmerged branch check removed - git branch -d/D handles this natively
    // The CLI provides a user-friendly confirmation prompt before calling this function
    let prefix = config.window_prefix();
    info!(
        branch = branch_name,
        delete_remote, keep_branch, "remove:cleanup start"
    );
    let cleanup_result = cleanup::cleanup(
        prefix,
        branch_name,
        &worktree_path,
        force,
        delete_remote,
        keep_branch,
        config,
    )?;

    // Navigate to the main branch window and close the target window
    cleanup::navigate_to_main_and_close(prefix, &main_branch, branch_name, &cleanup_result)?;

    Ok(RemoveResult {
        branch_removed: branch_name.to_string(),
    })
}
