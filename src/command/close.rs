use crate::multiplexer::{create_backend, detect_backend, util};
use crate::{config, git, sandbox};
use anyhow::{Context, Result, anyhow};

pub fn run(name: Option<&str>) -> Result<()> {
    let config = config::Config::load(None)?;
    let mux = create_backend(detect_backend());
    let prefix = config.window_prefix();

    // When no name is provided, prefer the current window name
    // This handles duplicate windows (e.g., wm:feature-2) correctly
    let (full_window_name, is_current_window) = match name {
        Some(name) => {
            // Explicit name provided - validate the worktree exists and get path
            let (path, _branch) = git::find_worktree(name).with_context(|| {
                format!(
                    "No worktree found with name '{}'. Use 'workmux list' to see available worktrees.",
                    name
                )
            })?;
            // Extract actual handle from worktree path (directory name)
            let handle = path.file_name().and_then(|n| n.to_str()).unwrap_or(name);
            let prefixed = util::prefixed(prefix, handle);
            let current_window = mux.current_window_name()?;
            let is_current = current_window.as_deref() == Some(&prefixed);
            (prefixed, is_current)
        }
        None => {
            // No name provided - check if we're in a workmux window
            if let Some(current) = mux.current_window_name()? {
                if current.starts_with(prefix) {
                    // We're in a workmux window, use it directly
                    (current.clone(), true)
                } else {
                    // Not in a workmux window, fall back to directory name
                    let handle = super::resolve_name(None)?;
                    (util::prefixed(prefix, &handle), false)
                }
            } else {
                // Not in tmux, use directory name
                let handle = super::resolve_name(None)?;
                (util::prefixed(prefix, &handle), false)
            }
        }
    };

    // Check if the window exists
    if !mux.window_exists_by_full_name(&full_window_name)? {
        return Err(anyhow!(
            "No active window found for '{}'. The worktree exists but has no open window.",
            full_window_name
        ));
    }

    // Stop any running containers for this worktree before killing the window.
    // We try unconditionally since sandbox may have been enabled via --sandbox flag.
    // Extract handle from full window name (e.g., "wm:feature-auth" -> "feature-auth")
    if let Some(handle) = full_window_name.strip_prefix(prefix) {
        sandbox::stop_containers_for_handle(handle, &config.sandbox);
    }

    if is_current_window {
        // Schedule the window close with a small delay so the command can complete
        mux.schedule_window_close(&full_window_name, std::time::Duration::from_millis(100))?;
    } else {
        // Kill the window directly
        mux.kill_window(&full_window_name)
            .context("Failed to close window")?;
        println!("✓ Closed window '{}' (worktree kept)", full_window_name);
    }

    Ok(())
}
