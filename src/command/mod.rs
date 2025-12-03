pub mod add;
pub mod args;
pub mod list;
pub mod merge;
pub mod open;
pub mod path;
pub mod remove;
pub mod set_window_status;

use anyhow::{Context, Result, anyhow};

use crate::{config::Config, workflow::SetupOptions};

/// Represents the different phases where hooks can be executed
pub enum HookPhase {
    PostCreate,
    PreDelete,
}

/// Announce that hooks are about to run, if applicable.
/// Returns true if the announcement was printed (hooks will run).
pub fn announce_hooks(config: &Config, options: Option<&SetupOptions>, phase: HookPhase) -> bool {
    match phase {
        HookPhase::PostCreate => {
            let should_run = options.is_some_and(|opts| opts.run_hooks)
                && config.post_create.as_ref().is_some_and(|v| !v.is_empty());

            if should_run {
                println!("Running setup commands...");
            }
            should_run
        }
        HookPhase::PreDelete => {
            let should_run = config.pre_delete.as_ref().is_some_and(|v| !v.is_empty());

            if should_run {
                println!("Running pre-delete commands...");
            }
            should_run
        }
    }
}

/// Resolve name from argument or current worktree directory.
pub fn resolve_name(arg: Option<&str>) -> Result<String> {
    match arg {
        Some(name) => Ok(name.to_string()),
        None => {
            let cwd = std::env::current_dir().context("Failed to get current directory")?;
            cwd.file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow!("Could not determine worktree name from current directory"))
        }
    }
}
