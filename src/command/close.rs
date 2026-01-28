use crate::config::TmuxTarget;
use crate::multiplexer::{create_backend, detect_backend, util};
use crate::{config, git, sandbox};
use anyhow::{Context, Result, anyhow};

pub fn run(name: Option<&str>) -> Result<()> {
    let config = config::Config::load(None)?;
    let mux = create_backend(detect_backend());
    let prefix = config.window_prefix();

    // Resolve the handle first to determine target mode
    let resolved_handle = match name {
        Some(h) => h.to_string(),
        None => super::resolve_name(None)?,
    };

    // Determine if this worktree was created as a session or window
    let is_session_mode = git::get_worktree_target(&resolved_handle) == TmuxTarget::Session;

    // When no name is provided, prefer the current window/session name
    // This handles duplicate windows/sessions (e.g., wm:feature-2) correctly
    let (full_target_name, is_current_target) = match name {
        Some(handle) => {
            // Explicit name provided - validate the worktree exists
            git::find_worktree(handle).with_context(|| {
                format!(
                    "No worktree found with name '{}'. Use 'workmux list' to see available worktrees.",
                    handle
                )
            })?;
            let prefixed = util::prefixed(prefix, handle);
            let is_current = if is_session_mode {
                // For sessions, check current session
                // TODO: Add current_session_name to Multiplexer trait
                let current_window = mux.current_window_name()?;
                current_window.as_deref() == Some(&prefixed)
            } else {
                let current_window = mux.current_window_name()?;
                current_window.as_deref() == Some(&prefixed)
            };
            (prefixed, is_current)
        }
        None => {
            // No name provided - check if we're in a workmux window/session
            if let Some(current) = mux.current_window_name()? {
                if current.starts_with(prefix) {
                    // We're in a workmux window, use it directly
                    (current.clone(), true)
                } else {
                    // Not in a workmux window, fall back to resolved handle
                    (util::prefixed(prefix, &resolved_handle), false)
                }
            } else {
                // Not in multiplexer, use resolved handle
                (util::prefixed(prefix, &resolved_handle), false)
            }
        }
    };

    let target_type = if is_session_mode { "session" } else { "window" };

    // Check if the window/session exists
    // For now, we only check windows - session support requires trait extension
    let target_exists = mux.window_exists_by_full_name(&full_target_name)?;

    if !target_exists {
        return Err(anyhow!(
            "No active {} found for '{}'. The worktree exists but has no open {}.",
            target_type,
            full_target_name,
            target_type
        ));
    }

    // Stop any running containers for this worktree before killing the window.
    // We try unconditionally since sandbox may have been enabled via --sandbox flag.
    // Extract handle from full target name (e.g., "wm:feature-auth" -> "feature-auth")
    if let Some(handle) = full_target_name.strip_prefix(prefix) {
        sandbox::stop_containers_for_handle(handle, &config.sandbox);
    }

    if is_current_target {
        // Schedule the close with a small delay so the command can complete
        mux.schedule_window_close(&full_target_name, std::time::Duration::from_millis(100))?;
    } else {
        // Kill the target directly
        mux.kill_window(&full_target_name)
            .context("Failed to close window")?;
        println!(
            "✓ Closed {} '{}' (worktree kept)",
            target_type, full_target_name
        );
    }

    Ok(())
}
