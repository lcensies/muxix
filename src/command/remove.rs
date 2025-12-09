use crate::workflow::WorkflowContext;
use crate::{config, git, spinner, workflow};
use anyhow::{Context, Result, anyhow};
use std::io::{self, Write};
use std::path::PathBuf;

/// User's choice when prompted about unmerged commits.
enum UserChoice {
    Confirmed, // User confirmed deletion
    Aborted,   // User aborted deletion
    NotNeeded, // No prompt needed (no unmerged commits)
}

pub fn run(name: Option<&str>, gone: bool, force: bool, keep_branch: bool) -> Result<()> {
    if gone {
        return run_gone(force, keep_branch);
    }

    run_single(name, force, keep_branch)
}

/// Remove a single worktree by name
fn run_single(name: Option<&str>, force: bool, keep_branch: bool) -> Result<()> {
    // Resolve name from argument or current worktree directory
    let input_name = super::resolve_name(name)?;

    // Smart resolution: try handle first, then branch name
    let (worktree_path, branch_name) = git::find_worktree(&input_name)
        .with_context(|| format!("No worktree found with name '{}'", input_name))?;

    // Derive handle from the worktree path (in case user provided branch name)
    let handle = worktree_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("Could not derive handle from worktree path"))?
        .to_string();

    // Validate removal safety and get effective force flag
    let effective_force =
        match validate_removal_safety(&handle, &worktree_path, &branch_name, force, keep_branch)? {
            Some(force_flag) => force_flag,
            None => return Ok(()), // User aborted
        };

    remove_worktree(&handle, effective_force, keep_branch)
}

/// Remove worktrees whose upstream remote branch has been deleted
fn run_gone(force: bool, keep_branch: bool) -> Result<()> {
    // Fetch with prune to update remote-tracking refs
    spinner::with_spinner("Fetching from remote", git::fetch_prune)?;

    let worktrees = git::list_worktrees()?;
    let main_branch = git::get_default_branch()?;
    let main_worktree_root = git::get_main_worktree_root()?;

    let gone_branches = git::get_gone_branches().unwrap_or_default();

    // Find worktrees whose upstream is gone
    let mut to_remove: Vec<(PathBuf, String, String)> = Vec::new();
    let mut skipped_uncommitted: Vec<String> = Vec::new();

    for (path, branch) in worktrees {
        // Skip main branch/worktree and detached HEAD
        if branch == main_branch || branch == "(detached)" {
            continue;
        }

        // Skip the main worktree itself
        if path == main_worktree_root {
            continue;
        }

        // Check if upstream is gone
        if !gone_branches.contains(&branch) {
            continue;
        }

        // Check for uncommitted changes
        if !force && path.exists() && git::has_uncommitted_changes(&path).unwrap_or(false) {
            skipped_uncommitted.push(branch);
            continue;
        }

        let handle = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&branch)
            .to_string();

        to_remove.push((path, branch, handle));
    }

    if to_remove.is_empty() && skipped_uncommitted.is_empty() {
        println!("No worktrees with gone upstreams found.");
        return Ok(());
    }

    if to_remove.is_empty() {
        println!("No worktrees to remove.");
        if !skipped_uncommitted.is_empty() {
            println!(
                "\nSkipped {} worktree(s) with uncommitted changes:",
                skipped_uncommitted.len()
            );
            for branch in &skipped_uncommitted {
                println!("  - {}", branch);
            }
            println!("\nUse --force to remove these anyway.");
        }
        return Ok(());
    }

    // Show what will be removed
    println!("The following worktrees have gone upstreams and will be removed:");
    for (_, branch, _) in &to_remove {
        println!("  - {}", branch);
    }

    if !skipped_uncommitted.is_empty() {
        println!(
            "\nSkipping {} worktree(s) with uncommitted changes:",
            skipped_uncommitted.len()
        );
        for branch in &skipped_uncommitted {
            println!("  - {}", branch);
        }
    }

    // Confirm with user unless --force
    if !force {
        print!(
            "\nAre you sure you want to remove {} worktree(s)? [y/N] ",
            to_remove.len()
        );
        io::stdout().flush().context("Failed to flush stdout")?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("Failed to read user input")?;

        if input.trim().to_lowercase() != "y" {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Execute removal
    let mut success_count = 0;
    let mut failed: Vec<(String, String)> = Vec::new();

    for (_, branch, handle) in to_remove {
        match remove_worktree(&handle, true, keep_branch) {
            Ok(()) => success_count += 1,
            Err(e) => failed.push((branch, e.to_string())),
        }
    }

    // Report results
    if success_count > 0 {
        println!("\n✓ Successfully removed {} worktree(s)", success_count);
    }

    if !failed.is_empty() {
        eprintln!("\nFailed to remove {} worktree(s):", failed.len());
        for (branch, error) in &failed {
            eprintln!("  - {}: {}", branch, error);
        }
    }

    Ok(())
}

/// Execute the actual worktree removal
fn remove_worktree(handle: &str, force: bool, keep_branch: bool) -> Result<()> {
    let config = config::Config::load(None)?;
    let context = WorkflowContext::new(config)?;

    super::announce_hooks(&context.config, None, super::HookPhase::PreDelete);

    let result = workflow::remove(handle, force, keep_branch, &context)
        .context("Failed to remove worktree")?;

    if keep_branch {
        println!(
            "✓ Removed worktree '{}' (branch '{}' kept)",
            handle, result.branch_removed
        );
    } else {
        println!(
            "✓ Removed worktree '{}' and branch '{}'",
            handle, result.branch_removed
        );
    }

    Ok(())
}

/// Validates whether it's safe to remove the branch/worktree.
/// Returns Some(force_flag) to proceed, or None if user aborted.
fn validate_removal_safety(
    handle: &str,
    worktree_path: &std::path::Path,
    branch_name: &str,
    force: bool,
    keep_branch: bool,
) -> Result<Option<bool>> {
    if force {
        return Ok(Some(true));
    }

    // First check for uncommitted changes (must be checked before unmerged prompt)
    // to avoid prompting user about unmerged commits only to error on uncommitted changes
    check_uncommitted_changes(worktree_path)?;

    // Check if we need to prompt for unmerged commits (only relevant when deleting the branch)
    if !keep_branch {
        match check_unmerged_commits(handle, branch_name)? {
            UserChoice::Confirmed => return Ok(Some(true)), // User confirmed - use force
            UserChoice::Aborted => return Ok(None),         // User aborted
            UserChoice::NotNeeded => {}                     // No unmerged commits
        }
    }

    Ok(Some(false))
}

/// Check for uncommitted changes in the worktree.
fn check_uncommitted_changes(worktree_path: &std::path::Path) -> Result<()> {
    if worktree_path.exists() {
        let has_changes = git::has_uncommitted_changes(worktree_path).with_context(|| {
            format!(
                "Failed to check for uncommitted changes in worktree at '{}'",
                worktree_path.display()
            )
        })?;

        if has_changes {
            return Err(anyhow!(
                "Worktree has uncommitted changes. Use --force to delete anyway."
            ));
        }
    }

    Ok(())
}

/// Check for unmerged commits and prompt user for confirmation.
fn check_unmerged_commits(handle: &str, branch_name: &str) -> Result<UserChoice> {
    // Try to get the stored base branch, fall back to default branch
    let base = git::get_branch_base(branch_name)
        .ok()
        .unwrap_or_else(|| git::get_default_branch().unwrap_or_else(|_| "main".to_string()));

    // Get the merge base with fallback if the stored base is invalid
    let base_branch = match git::get_merge_base(&base) {
        Ok(b) => b,
        Err(_) => {
            let default_main = git::get_default_branch().context("Failed to get default branch")?;
            eprintln!(
                "Warning: Could not resolve base '{}'; falling back to '{}'",
                base, default_main
            );
            git::get_merge_base(&default_main)
                .with_context(|| format!("Failed to get merge base for '{}'", default_main))?
        }
    };

    let unmerged_branches = git::get_unmerged_branches(&base_branch)
        .with_context(|| format!("Failed to get unmerged branches for base '{}'", base_branch))?;

    let has_unmerged = unmerged_branches.contains(branch_name);

    if has_unmerged {
        prompt_unmerged_confirmation(handle, branch_name, &base_branch, &base)
    } else {
        Ok(UserChoice::NotNeeded)
    }
}

/// Prompt user to confirm deletion of branch with unmerged commits.
fn prompt_unmerged_confirmation(
    handle: &str,
    branch_name: &str,
    base_branch: &str,
    base: &str,
) -> Result<UserChoice> {
    println!(
        "This will delete the worktree '{}', tmux window, and local branch '{}'.",
        handle, branch_name
    );
    println!(
        "Warning: Branch '{}' has commits that are not merged into '{}' (base: '{}').",
        branch_name, base_branch, base
    );
    println!("This action cannot be undone.");
    print!("Are you sure you want to continue? [y/N] ");

    // Flush stdout to ensure the prompt is displayed before reading input
    io::stdout().flush().context("Failed to flush stdout")?;

    let mut confirmation = String::new();
    io::stdin()
        .read_line(&mut confirmation)
        .context("Failed to read user confirmation")?;

    if confirmation.trim().to_lowercase() == "y" {
        Ok(UserChoice::Confirmed)
    } else {
        println!("Aborted.");
        Ok(UserChoice::Aborted)
    }
}
