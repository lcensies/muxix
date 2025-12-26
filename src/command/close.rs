use crate::{config, git, tmux};
use anyhow::{Context, Result, anyhow};

pub fn run(name: Option<&str>) -> Result<()> {
    let config = config::Config::load(None)?;
    let prefix = config.window_prefix();

    // Resolve the handle (worktree name)
    let handle = super::resolve_name(name)?;

    // Validate the worktree exists
    git::find_worktree(&handle).with_context(|| {
        format!(
            "No worktree found with name '{}'. Use 'workmux list' to see available worktrees.",
            handle
        )
    })?;

    // Check if the tmux window exists
    if !tmux::window_exists(prefix, &handle)? {
        return Err(anyhow!(
            "No active tmux window found for '{}'. The worktree exists but has no open window.",
            handle
        ));
    }

    // Check if we're inside the window we're about to close
    let prefixed_name = tmux::prefixed(prefix, &handle);
    let current_window = tmux::current_window_name()?;
    let is_current_window = current_window.as_deref() == Some(&prefixed_name);

    if is_current_window {
        // Schedule the window close with a small delay so the command can complete
        tmux::schedule_window_close(prefix, &handle, std::time::Duration::from_millis(100))?;
    } else {
        // Kill the window directly
        tmux::kill_window(prefix, &handle).context("Failed to close tmux window")?;
        println!("✓ Closed window '{}' (worktree kept)", prefixed_name);
    }

    Ok(())
}
