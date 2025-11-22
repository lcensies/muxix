use anyhow::{Context, Result, anyhow};

use crate::{git, tmux};
use tracing::info;

use super::context::WorkflowContext;
use super::setup;
use super::types::{CreateResult, SetupOptions};

/// Open a tmux window for an existing worktree
pub fn open(
    branch_name: &str,
    context: &WorkflowContext,
    options: SetupOptions,
) -> Result<CreateResult> {
    info!(
        branch = branch_name,
        run_hooks = options.run_hooks,
        run_file_ops = options.run_file_ops,
        "open:start"
    );

    // Validate pane config before any other operations
    if let Some(panes) = &context.config.panes {
        crate::config::validate_panes_config(panes)?;
    }

    // Pre-flight checks
    context.ensure_tmux_running()?;

    if tmux::window_exists(&context.prefix, branch_name)? {
        return Err(anyhow!(
            "A tmux window named '{}' already exists. To switch to it, run: tmux select-window -t '{}'",
            branch_name,
            tmux::prefixed(&context.prefix, branch_name)
        ));
    }

    // This command requires the worktree to already exist
    let worktree_path = git::get_worktree_path(branch_name).with_context(|| {
        format!(
            "No worktree found for branch '{}'. Use 'workmux add {}' to create it.",
            branch_name, branch_name
        )
    })?;

    // Setup the environment
    let result =
        setup::setup_environment(branch_name, &worktree_path, &context.config, &options, None)?;
    info!(
        branch = branch_name,
        path = %result.worktree_path.display(),
        hooks_run = result.post_create_hooks_run,
        "open:completed"
    );
    Ok(result)
}
