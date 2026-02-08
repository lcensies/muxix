use crate::config::MergeStrategy;
use crate::multiplexer::{create_backend, detect_backend};
use crate::workflow::WorkflowContext;
use crate::{config, workflow};
use anyhow::{Context, Result};

#[allow(clippy::too_many_arguments)]
pub fn run(
    name: Option<&str>,
    into_branch: Option<&str>,
    ignore_uncommitted: bool,
    mut rebase: bool,
    mut squash: bool,
    keep: bool,
    no_verify: bool,
    notification: bool,
) -> Result<()> {
    // Inside a sandbox guest, route through RPC to the host supervisor
    if crate::sandbox::guest::is_sandbox_guest() {
        let name_to_merge = super::resolve_name(name)?;
        return run_via_rpc(&name_to_merge, into_branch, rebase, ignore_uncommitted);
    }

    let config = config::Config::load(None)?;

    // Apply default strategy from config if no CLI flags are provided
    if !rebase
        && !squash
        && let Some(strategy) = config.merge_strategy
    {
        match strategy {
            MergeStrategy::Rebase => rebase = true,
            MergeStrategy::Squash => squash = true,
            MergeStrategy::Merge => {}
        }
    }

    // Resolve name from argument or current directory
    // Note: Must be done BEFORE creating WorkflowContext (which may change CWD)
    let name_to_merge = super::resolve_name(name)?;

    let mux = create_backend(detect_backend());
    let context = WorkflowContext::new(config, mux, None)?;

    // Announce pre-merge hooks if any (unless --no-verify is passed)
    if !no_verify {
        super::announce_hooks(&context.config, None, super::HookPhase::PreMerge);
    }

    // Only announce pre-remove hooks if we're actually going to run cleanup
    if !keep {
        super::announce_hooks(&context.config, None, super::HookPhase::PreRemove);
    }

    let result = workflow::merge(
        &name_to_merge,
        into_branch,
        ignore_uncommitted,
        rebase,
        squash,
        keep,
        no_verify,
        notification,
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
        println!("Worktree, window, and branch kept");
    } else {
        println!(
            "✓ Successfully merged and cleaned up '{}'",
            result.branch_merged
        );
    }

    Ok(())
}

/// Run merge via RPC when inside a sandbox guest.
fn run_via_rpc(
    name: &str,
    into: Option<&str>,
    rebase: bool,
    ignore_uncommitted: bool,
) -> Result<()> {
    use crate::sandbox::rpc::{RpcClient, RpcRequest, RpcResponse};

    let mut client = RpcClient::from_env()?;
    let response = client.call(&RpcRequest::Merge {
        name: name.to_string(),
        into: into.map(|s| s.to_string()),
        rebase,
        ignore_uncommitted,
    })?;

    match response {
        RpcResponse::Ok => {
            println!("Merge completed successfully");
            Ok(())
        }
        RpcResponse::Output { message } => {
            print!("{}", message);
            Ok(())
        }
        RpcResponse::Error { message } => {
            anyhow::bail!("{}", message);
        }
        _ => Ok(()),
    }
}
