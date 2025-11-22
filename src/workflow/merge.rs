use anyhow::{Context, Result, anyhow};

use crate::git;
use tracing::{debug, info};

use super::cleanup;
use super::context::WorkflowContext;
use super::types::MergeResult;

/// Merge a branch into the main branch and clean up
pub fn merge(
    branch_name: &str,
    ignore_uncommitted: bool,
    delete_remote: bool,
    rebase: bool,
    squash: bool,
    context: &WorkflowContext,
) -> Result<MergeResult> {
    info!(
        branch = branch_name,
        ignore_uncommitted, delete_remote, rebase, squash, "merge:start"
    );

    // Change CWD to main worktree to prevent errors if the command is run from within
    // the worktree that is about to be deleted.
    context.chdir_to_main_worktree()?;

    let branch_to_merge = branch_name;

    // Get worktree path for the branch to be merged
    let worktree_path = git::get_worktree_path(branch_to_merge)
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

    if branch_to_merge == context.main_branch {
        return Err(anyhow!("Cannot merge the main branch into itself."));
    }
    debug!(
        branch = branch_to_merge,
        main = &context.main_branch,
        "merge:main branch resolved"
    );

    // Safety check: Abort if the main worktree has uncommitted changes
    if git::has_uncommitted_changes(&context.main_worktree_root)? {
        return Err(anyhow!(
            "Main worktree has uncommitted changes. Please commit or stash them before merging."
        ));
    }

    // Explicitly switch to the main branch to ensure correct merge target
    git::switch_branch_in_worktree(&context.main_worktree_root, &context.main_branch)?;

    if rebase {
        // Rebase the feature branch on top of main inside its own worktree.
        // This is where conflicts will be detected.
        println!(
            "Rebasing '{}' onto '{}'...",
            &branch_to_merge, &context.main_branch
        );
        info!(
            branch = branch_to_merge,
            base = &context.main_branch,
            "merge:rebase start"
        );
        git::rebase_branch_onto_base(&worktree_path, &context.main_branch).with_context(|| {
            format!(
                "Rebase failed, likely due to conflicts.\n\n\
                Please resolve them manually inside the worktree at '{}'.\n\
                Then, run 'git rebase --continue' to proceed or 'git rebase --abort' to cancel.",
                worktree_path.display()
            )
        })?;

        // After a successful rebase, merge into main. This will be a fast-forward.
        git::merge_in_worktree(&context.main_worktree_root, branch_to_merge)
            .context("Failed to merge rebased branch. This should have been a fast-forward.")?;
        info!(branch = branch_to_merge, "merge:fast-forward complete");
    } else if squash {
        // Perform the squash merge. This stages all changes from the feature branch but does not commit.
        git::merge_squash_in_worktree(&context.main_worktree_root, branch_to_merge)
            .context("Failed to perform squash merge")?;

        // Prompt the user to provide a commit message for the squashed changes.
        println!("Staged squashed changes. Please provide a commit message in your editor.");
        git::commit_with_editor(&context.main_worktree_root)
            .context("Failed to commit squashed changes. You may need to commit them manually.")?;
        info!(branch = branch_to_merge, "merge:squash merge committed");
    } else {
        // Default merge commit workflow
        git::merge_in_worktree(&context.main_worktree_root, branch_to_merge)
            .context("Failed to merge branch")?;
        info!(branch = branch_to_merge, "merge:standard merge complete");
    }

    // Always force cleanup after a successful merge
    info!(
        branch = branch_to_merge,
        delete_remote, "merge:cleanup start"
    );
    let cleanup_result = cleanup::cleanup(
        context,
        branch_to_merge,
        &worktree_path,
        true,
        delete_remote,
        false, // keep_branch: always delete when merging
    )?;

    // Navigate to the main branch window and close the target window
    cleanup::navigate_to_main_and_close(
        &context.prefix,
        &context.main_branch,
        branch_to_merge,
        &cleanup_result,
    )?;

    Ok(MergeResult {
        branch_merged: branch_to_merge.to_string(),
        main_branch: context.main_branch.clone(),
        had_staged_changes,
    })
}
