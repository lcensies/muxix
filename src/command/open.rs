use crate::workflow::{SetupOptions, WorkflowContext};
use crate::{config, workflow};
use anyhow::{Context, Result};

pub fn run(branch_name: &str, run_hooks: bool, force_files: bool) -> Result<()> {
    let config = config::Config::load(None)?;
    let context = WorkflowContext::new(config)?;

    // Construct setup options (pane commands always run on open)
    let options = SetupOptions::new(run_hooks, force_files, true);

    super::announce_hooks(&context.config, Some(&options), super::HookPhase::PostCreate);

    let result = workflow::open(branch_name, &context, options)
        .context("Failed to open worktree environment")?;

    if result.post_create_hooks_run > 0 {
        println!("✓ Setup complete");
    }

    println!(
        "✓ Successfully opened tmux window for '{}'\n  Worktree: {}",
        result.branch_name,
        result.worktree_path.display()
    );

    Ok(())
}
