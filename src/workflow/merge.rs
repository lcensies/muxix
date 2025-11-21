use anyhow::{Context, Result, anyhow};

use crate::{config, git};
use tracing::{debug, info};

use super::cleanup;
use super::types::MergeResult;

/// Merge a branch into the main branch and clean up
pub fn merge(
    branch_name: Option<&str>,
    ignore_uncommitted: bool,
    delete_remote: bool,
    rebase: bool,
    squash: bool,
    config: &config::Config,
) -> Result<MergeResult> {
    info!(
        branch = ?branch_name,
        ignore_uncommitted,
        delete_remote,
        rebase,
        squash,
        "merge:start"
    );
    // Pre-flight checks
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    // Change CWD to main worktree to prevent errors if the command is run from within
    // the worktree that is about to be deleted.
    let main_worktree_root = git::get_main_worktree_root()
        .context("Could not find main worktree to run merge operations")?;
    debug!(safe_cwd = %main_worktree_root.display(), "merge:changing to main worktree");
    std::env::set_current_dir(&main_worktree_root).with_context(|| {
        format!(
            "Could not change directory to '{}'",
            main_worktree_root.display()
        )
    })?;

    // Determine the branch to merge
    let branch_to_merge = if let Some(name) = branch_name {
        name.to_string()
    } else {
        // Running from within a worktree - get current branch
        git::get_current_branch().context("Failed to get current branch")?
    };

    // Get worktree path for the branch to be merged
    let worktree_path = git::get_worktree_path(&branch_to_merge)
        .with_context(|| format!("No worktree found for branch '{}'", branch_to_merge))?;
    debug!(
        branch = branch_to_merge,
        path = %worktree_path.display(),
        "merge:worktree resolved"
    );

    // Handle changes in the source worktree
    if git::has_unstaged_changes(&worktree_path)? && !ignore_uncommitted {
        return Err(anyhow!(
            "Worktree for '{}' has unstaged changes. Please stage or stash them, or use --ignore-uncommitted.",
            branch_to_merge
        ));
    }

    let had_staged_changes = git::has_staged_changes(&worktree_path)?;
    if had_staged_changes && !ignore_uncommitted {
        // Commit using git's editor (respects $EDITOR or git config)
        info!(path = %worktree_path.display(), "merge:committing staged changes");
        git::commit_with_editor(&worktree_path).context("Failed to commit staged changes")?;
    }

    // Get the main branch (from config or auto-detect)
    let main_branch = config
        .main_branch
        .as_ref()
        .map(|s| Ok(s.clone()))
        .unwrap_or_else(git::get_default_branch)
        .context("Failed to determine the main branch. Specify it in .workmux.yaml")?;

    if branch_to_merge == main_branch {
        return Err(anyhow!("Cannot merge the main branch into itself."));
    }
    debug!(
        branch = branch_to_merge,
        main = &main_branch,
        "merge:main branch resolved"
    );

    // Get the main worktree path. This is the canonical, non-linked worktree.
    let main_worktree_path =
        git::get_main_worktree_root().context("Failed to find the main worktree")?;

    // Safety check: Abort if the main worktree has uncommitted changes
    if git::has_uncommitted_changes(&main_worktree_path)? {
        return Err(anyhow!(
            "Main worktree has uncommitted changes. Please commit or stash them before merging."
        ));
    }

    // Explicitly switch to the main branch to ensure correct merge target
    git::switch_branch_in_worktree(&main_worktree_path, &main_branch)?;

    if rebase {
        // Rebase the feature branch on top of main inside its own worktree.
        // This is where conflicts will be detected.
        println!("Rebasing '{}' onto '{}'...", &branch_to_merge, &main_branch);
        info!(
            branch = branch_to_merge,
            base = &main_branch,
            "merge:rebase start"
        );
        git::rebase_branch_onto_base(&worktree_path, &main_branch).with_context(|| {
            format!(
                "Rebase failed, likely due to conflicts.\n\n\
                Please resolve them manually inside the worktree at '{}'.\n\
                Then, run 'git rebase --continue' to proceed or 'git rebase --abort' to cancel.",
                worktree_path.display()
            )
        })?;

        // After a successful rebase, merge into main. This will be a fast-forward.
        git::merge_in_worktree(&main_worktree_path, &branch_to_merge)
            .context("Failed to merge rebased branch. This should have been a fast-forward.")?;
        info!(branch = branch_to_merge, "merge:fast-forward complete");
    } else if squash {
        // Perform the squash merge. This stages all changes from the feature branch but does not commit.
        git::merge_squash_in_worktree(&main_worktree_path, &branch_to_merge)
            .context("Failed to perform squash merge")?;

        // Prompt the user to provide a commit message for the squashed changes.
        println!("Staged squashed changes. Please provide a commit message in your editor.");
        git::commit_with_editor(&main_worktree_path)
            .context("Failed to commit squashed changes. You may need to commit them manually.")?;
        info!(branch = branch_to_merge, "merge:squash merge committed");
    } else {
        // Default merge commit workflow
        git::merge_in_worktree(&main_worktree_path, &branch_to_merge)
            .context("Failed to merge branch")?;
        info!(branch = branch_to_merge, "merge:standard merge complete");
    }

    // Always force cleanup after a successful merge
    let prefix = config.window_prefix();
    info!(
        branch = branch_to_merge,
        delete_remote, "merge:cleanup start"
    );
    let cleanup_result = cleanup::cleanup(
        prefix,
        &branch_to_merge,
        &worktree_path,
        true,
        delete_remote,
        false, // keep_branch: always delete when merging
        config,
    )?;

    // Navigate to the main branch window and close the target window
    cleanup::navigate_to_main_and_close(prefix, &main_branch, &branch_to_merge, &cleanup_result)?;

    Ok(MergeResult {
        branch_merged: branch_to_merge,
        main_branch,
        had_staged_changes,
    })
}
