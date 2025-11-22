use crate::workflow::WorkflowContext;
use crate::{config, workflow};
use anyhow::{Context, Result};

pub fn run(
    branch_name: Option<&str>,
    ignore_uncommitted: bool,
    delete_remote: bool,
    rebase: bool,
    squash: bool,
    keep: bool,
) -> Result<()> {
    let config = config::Config::load(None)?;

    // Resolve branch name from argument or current branch
    // Note: Must be done BEFORE creating WorkflowContext (which may change CWD)
    let branch_to_merge = super::resolve_branch(branch_name, "merge")?;

    let context = WorkflowContext::new(config)?;

    // Only announce pre-delete hooks if we're actually going to run cleanup
    if !keep {
        super::announce_hooks(&context.config, None, super::HookPhase::PreDelete);
    }

    let result = workflow::merge(
        &branch_to_merge,
        ignore_uncommitted,
        delete_remote,
        rebase,
        squash,
        keep,
        &context,
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

    if keep {
        println!("Worktree, window, and branch kept (--keep)");
    } else {
        println!(
            "✓ Successfully merged and cleaned up '{}'",
            result.branch_merged
        );
    }

    Ok(())
}
