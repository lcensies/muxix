use crate::{config, workflow};
use anyhow::{Context, Result};

pub fn run(
    branch_name: Option<&str>,
    ignore_uncommitted: bool,
    delete_remote: bool,
    rebase: bool,
    squash: bool,
) -> Result<()> {
    let config = config::Config::load(None)?;

    // Resolve branch name from argument or current branch
    let branch_to_merge = super::resolve_branch(branch_name, "merge")?;

    super::announce_hooks(&config, None, super::HookPhase::PreDelete);

    let result = workflow::merge(
        Some(&branch_to_merge),
        ignore_uncommitted,
        delete_remote,
        rebase,
        squash,
        &config,
    )
    .context("Failed to merge worktree")?;

    if result.had_staged_changes {
        println!("✓ Committed staged changes");
    }

    println!(
        "Merging '{}' into '{}'...",
        result.branch_merged, result.main_branch
    );
    println!("✓ Merged '{}'", result.branch_merged);

    println!(
        "✓ Successfully merged and cleaned up '{}'",
        result.branch_merged
    );

    Ok(())
}
