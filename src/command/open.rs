use crate::workflow::SetupOptions;
use crate::{config, workflow};
use anyhow::{Context, Result};

pub fn run(branch_name: &str, run_hooks: bool, force_files: bool) -> Result<()> {
    let config = config::Config::load(None)?;

    // Construct setup options (pane commands always run on open)
    let options = SetupOptions::new(run_hooks, force_files, true);

    if options.run_hooks && config.post_create.as_ref().is_some_and(|v| !v.is_empty()) {
        println!("Running setup commands...");
    }

    let result = workflow::open(branch_name, &config, options)
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
