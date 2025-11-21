use crate::{config, git, workflow};
use anyhow::{Context, Result, anyhow};
use std::io::{self, Write};

pub fn run(
    branch_name: Option<&str>,
    mut force: bool,
    delete_remote: bool,
    keep_branch: bool,
) -> Result<()> {
    // Determine the branch to remove
    // Note: If running without branch name, we must get current branch BEFORE workflow::remove
    // changes the CWD (since it moves to main worktree for safety)
    let branch_to_remove = if let Some(name) = branch_name {
        name.to_string()
    } else {
        // Running from within a worktree - get current branch
        git::get_current_branch().context("Failed to get current branch")?
    };

    // Handle user confirmation prompt if needed (before calling workflow)
    if !force {
        // First check for uncommitted changes (must be checked before unmerged prompt)
        // to avoid prompting user about unmerged commits only to error on uncommitted changes
        if let Ok(worktree_path) = git::get_worktree_path(&branch_to_remove)
            && worktree_path.exists()
            && git::has_uncommitted_changes(&worktree_path)?
        {
            return Err(anyhow!(
                "Worktree has uncommitted changes. Use --force to delete anyway."
            ));
        }

        // Check if we need to prompt for unmerged commits (only relevant when deleting the branch)
        if !keep_branch {
            // Try to get the stored base branch, fall back to default branch
            let base = git::get_branch_base(&branch_to_remove)
                .ok()
                .unwrap_or_else(|| {
                    git::get_default_branch().unwrap_or_else(|_| "main".to_string())
                });

            // Get the merge base with fallback if the stored base is invalid
            let base_branch = match git::get_merge_base(&base) {
                Ok(b) => b,
                Err(_) => {
                    let default_main = git::get_default_branch()?;
                    eprintln!(
                        "Warning: Could not resolve base '{}'; falling back to '{}'",
                        base, default_main
                    );
                    git::get_merge_base(&default_main)?
                }
            };

            let unmerged_branches = git::get_unmerged_branches(&base_branch)?;
            let has_unmerged = unmerged_branches.contains(&branch_to_remove);

            if has_unmerged {
                println!(
                    "This will delete the worktree, tmux window, and local branch for '{}'.",
                    branch_to_remove
                );
                if delete_remote {
                    println!("The remote branch will also be deleted.");
                }
                println!(
                    "Warning: Branch '{}' has commits that are not merged into '{}' (base: '{}').",
                    branch_to_remove, base_branch, base
                );
                println!("This action cannot be undone.");
                print!("Are you sure you want to continue? [y/N] ");

                // Flush stdout to ensure the prompt is displayed before reading input
                io::stdout().flush()?;

                let mut confirmation = String::new();
                io::stdin().read_line(&mut confirmation)?;

                if confirmation.trim().to_lowercase() != "y" {
                    println!("Aborted.");
                    return Ok(());
                }

                // User confirmed deletion of unmerged branch - treat as force for git operations
                // This is safe because we already verified there are no uncommitted changes above
                force = true;
            }
        }
    }

    let config = config::Config::load(None)?;

    // Print status if there are pre-delete hooks
    if config.pre_delete.as_ref().is_some_and(|v| !v.is_empty()) {
        println!("Running pre-delete commands...");
    }

    let result = workflow::remove(
        &branch_to_remove,
        force,
        delete_remote,
        keep_branch,
        &config,
    )
    .context("Failed to remove worktree")?;

    if keep_branch {
        println!(
            "✓ Successfully removed worktree for branch '{}'. The local branch was kept.",
            result.branch_removed
        );
    } else {
        println!(
            "✓ Successfully removed worktree and branch '{}'",
            result.branch_removed
        );
    }

    Ok(())
}
