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
    no_hooks: bool,
    notification: bool,
) -> Result<()> {
    // Inside a sandbox guest, route through RPC to the host supervisor
    if crate::sandbox::guest::is_sandbox_guest() {
        let name_to_merge = super::resolve_name(name)?;
        return run_via_rpc(
            &name_to_merge,
            into_branch,
            rebase,
            squash,
            ignore_uncommitted,
            keep,
            no_verify,
            no_hooks,
            notification,
        );
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

    let skip_hooks = no_verify || no_hooks;

    // Announce pre-merge hooks if any (unless hooks are skipped)
    if !skip_hooks {
        super::announce_hooks(&context.config, None, super::HookPhase::PreMerge);
    }

    // Only announce pre-remove hooks if we're actually going to run cleanup
    if !keep && !no_hooks {
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
        no_hooks,
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
#[allow(clippy::too_many_arguments)]
fn run_via_rpc(
    name: &str,
    into: Option<&str>,
    rebase: bool,
    squash: bool,
    ignore_uncommitted: bool,
    keep: bool,
    no_verify: bool,
    no_hooks: bool,
    notification: bool,
) -> Result<()> {
    use crate::sandbox::rpc::{RpcClient, RpcRequest, RpcResponse};
    use std::io::Write;

    let mut client = RpcClient::from_env()?;
    client.send(&RpcRequest::Merge {
        name: name.to_string(),
        into: into.map(|s| s.to_string()),
        rebase,
        squash,
        ignore_uncommitted,
        keep,
        no_verify,
        no_hooks,
        notification,
    })?;

    // Read streaming responses until we get a terminal Ok or Error
    loop {
        let response = client.recv()?;
        match response {
            RpcResponse::Output { message } => {
                print!("{}", message);
                std::io::stdout().flush().ok();
            }
            RpcResponse::Ok => return Ok(()),
            RpcResponse::Error { message } => {
                anyhow::bail!("{}", message);
            }
            _ => {}
        }
    }
}
