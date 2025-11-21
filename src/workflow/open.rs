use anyhow::{Context, Result, anyhow};

use crate::{config, git, tmux};
use tracing::info;

use super::setup;
use super::types::{CreateResult, SetupOptions};

/// Open a tmux window for an existing worktree
pub fn open(
    branch_name: &str,
    config: &config::Config,
    options: SetupOptions,
) -> Result<CreateResult> {
    info!(
        branch = branch_name,
        run_hooks = options.run_hooks,
        run_file_ops = options.run_file_ops,
        "open:start"
    );

    // Validate pane config before any other operations
    if let Some(panes) = &config.panes {
        config::validate_panes_config(panes)?;
    }

    // Pre-flight checks
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    if !tmux::is_running()? {
        return Err(anyhow!(
            "tmux is not running. Please start a tmux session first."
        ));
    }

    let prefix = config.window_prefix();
    if tmux::window_exists(prefix, branch_name)? {
        return Err(anyhow!(
            "A tmux window named '{}' already exists. To switch to it, run: tmux select-window -t '{}'",
            branch_name,
            tmux::prefixed(prefix, branch_name)
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
    let result = setup::setup_environment(branch_name, &worktree_path, config, &options, None)?;
    info!(
        branch = branch_name,
        path = %result.worktree_path.display(),
        hooks_run = result.post_create_hooks_run,
        "open:completed"
    );
    Ok(result)
}
