use anyhow::{Context, Result};
use regex::Regex;
use std::path::Path;
use std::time::SystemTime;
use std::{thread, time::Duration};

use crate::config::TmuxTarget;
use crate::multiplexer::{Multiplexer, util::prefixed};
use crate::{cmd, git};
use tracing::{debug, info, warn};

// Re-export for use by other modules in the workflow
pub use git::get_worktree_target;

use super::context::WorkflowContext;
use super::types::{CleanupResult, DeferredCleanup};

const WINDOW_CLOSE_DELAY_MS: u64 = 300;

/// Best-effort recursive deletion of directory contents.
/// Used to ensure files are removed even if the directory itself is locked (e.g., CWD).
fn remove_dir_contents(path: &Path) {
    if !path.exists() {
        return;
    }

    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
        if is_dir {
            let _ = std::fs::remove_dir_all(&entry_path);
        } else {
            let _ = std::fs::remove_file(&entry_path);
        }
    }
}

/// Find all windows matching the base handle pattern (including duplicates).
/// Matches: {prefix}{handle} and {prefix}{handle}-{N}
fn find_matching_windows(mux: &dyn Multiplexer, prefix: &str, handle: &str) -> Result<Vec<String>> {
    let all_windows = mux.get_all_window_names()?;
    let base_name = prefixed(prefix, handle);
    let escaped_base = regex::escape(&base_name);
    let pattern = format!(r"^{}(-\d+)?$", escaped_base);
    let re = Regex::new(&pattern).expect("Invalid regex pattern");

    let matching: Vec<String> = all_windows.into_iter().filter(|w| re.is_match(w)).collect();

    Ok(matching)
}

/// Check if the current window/session matches the base handle pattern (including duplicates).
fn is_inside_matching_target(
    mux: &dyn Multiplexer,
    prefix: &str,
    handle: &str,
    is_session_mode: bool,
) -> Result<Option<String>> {
    let current_name = if is_session_mode {
        mux.current_session()
    } else {
        mux.current_window_name()?
    };

    let current_name = match current_name {
        Some(name) => name,
        None => return Ok(None),
    };

    let base_name = prefixed(prefix, handle);
    let escaped_base = regex::escape(&base_name);
    let pattern = format!(r"^{}(-\d+)?$", escaped_base);
    let re = Regex::new(&pattern).expect("Invalid regex pattern");

    if re.is_match(&current_name) {
        Ok(Some(current_name))
    } else {
        Ok(None)
    }
}

/// Centralized function to clean up tmux and git resources.
/// `branch_name` is used for git operations (branch deletion).
/// `handle` is used for tmux operations (window/session lookup/kill).
pub fn cleanup(
    context: &WorkflowContext,
    branch_name: &str,
    handle: &str,
    worktree_path: &Path,
    force: bool,
    keep_branch: bool,
    no_hooks: bool,
) -> Result<CleanupResult> {
    // Determine if this worktree was created as a session or window
    let target = get_worktree_target(handle);
    let is_session_mode = target == TmuxTarget::Session;

    info!(
        branch = branch_name,
        handle = handle,
        path = %worktree_path.display(),
        force,
        keep_branch,
        is_session_mode,
        "cleanup:start"
    );
    // Change the CWD to main worktree before any destructive operations.
    // This prevents "Unable to read current working directory" errors when the command
    // is run from within the worktree being deleted.
    context.chdir_to_main_worktree()?;

    let mux_running = context.mux.is_running().unwrap_or(false);

    // Check if we're running inside ANY matching target (original or duplicate)
    let current_matching_target = if mux_running {
        is_inside_matching_target(
            context.mux.as_ref(),
            &context.prefix,
            handle,
            is_session_mode,
        )?
    } else {
        None
    };
    let running_inside_target = current_matching_target.is_some();

    let mut result = CleanupResult {
        tmux_window_killed: false,
        worktree_removed: false,
        local_branch_deleted: false,
        window_to_close_later: None,
        trash_path_to_delete: None,
        deferred_cleanup: None,
    };

    // Helper closure to perform the actual filesystem and git cleanup.
    // This avoids code duplication while enforcing the correct operational order.
    let perform_fs_git_cleanup = |result: &mut CleanupResult| -> Result<()> {
        // Run pre-remove hooks before removing the worktree directory.
        // Skip if the worktree directory doesn't exist (e.g., user manually deleted it).
        // Skip if --no-hooks is set (e.g., RPC-triggered merge).
        if worktree_path.exists() && !no_hooks {
            if let Some(pre_remove_hooks) = &context.config.pre_remove {
                info!(
                    branch = branch_name,
                    count = pre_remove_hooks.len(),
                    "cleanup:running pre-remove hooks"
                );
                // Resolve absolute paths for environment variables.
                // canonicalize() ensures symlinks are resolved and paths are absolute.
                let abs_worktree_path = worktree_path
                    .canonicalize()
                    .unwrap_or_else(|_| worktree_path.to_path_buf());
                let abs_project_root = context
                    .main_worktree_root
                    .canonicalize()
                    .unwrap_or_else(|_| context.main_worktree_root.clone());
                let worktree_path_str = abs_worktree_path.to_string_lossy();
                let project_root_str = abs_project_root.to_string_lossy();
                let hook_env = [
                    ("WORKMUX_HANDLE", handle),
                    ("WM_HANDLE", handle),
                    ("WM_WORKTREE_PATH", worktree_path_str.as_ref()),
                    ("WM_PROJECT_ROOT", project_root_str.as_ref()),
                ];
                for command in pre_remove_hooks {
                    // Run the hook with the worktree path as the working directory.
                    // This allows for relative paths like `node_modules` in the command.
                    cmd::shell_command_with_env(command, worktree_path, &hook_env).with_context(
                        || format!("Failed to run pre-remove command: '{}'", command),
                    )?;
                }
            }
        } else {
            debug!(
                path = %worktree_path.display(),
                "cleanup:skipping pre-remove hooks, worktree directory does not exist"
            );
        }

        // Track the trash path for best-effort deletion at the end
        let mut trash_path: Option<std::path::PathBuf> = None;

        // 1. Rename the worktree directory to a trash location.
        // This immediately frees the original path for reuse, even if a shell process
        // still has it as CWD (the shell's CWD moves with the rename).
        // This fixes a race condition where running `workmux remove` from inside the
        // target tmux window could leave the directory behind.
        if worktree_path.exists() {
            let parent = worktree_path.parent().unwrap_or_else(|| Path::new("."));
            let dir_name = worktree_path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid worktree path: no directory name"))?;

            // Generate unique trash name: .workmux_trash_<name>_<timestamp>
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let trash_name = format!(
                ".workmux_trash_{}_{}",
                dir_name.to_string_lossy(),
                timestamp
            );
            let target_trash_path = parent.join(&trash_name);

            debug!(
                from = %worktree_path.display(),
                to = %target_trash_path.display(),
                "cleanup:renaming worktree to trash"
            );

            std::fs::rename(worktree_path, &target_trash_path).with_context(|| {
                format!(
                    "Failed to rename worktree directory to trash location '{}'. \
                    Please close any terminals or editors using this directory and try again.",
                    target_trash_path.display()
                )
            })?;

            trash_path = Some(target_trash_path);
            result.worktree_removed = true;
            info!(branch = branch_name, path = %worktree_path.display(), "cleanup:worktree directory removed");
        }

        // Clean up prompt files (handles both legacy fixed names and timestamped names)
        // Matches: workmux-prompt-{name}.md and workmux-prompt-{name}-{timestamp}.md
        let temp_dir = std::env::temp_dir();
        let prefix = format!("workmux-prompt-{}", branch_name);
        if let Ok(entries) = std::fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(filename) = path.file_name().and_then(|n| n.to_str())
                    && filename.starts_with(&prefix)
                    && filename.ends_with(".md")
                {
                    if let Err(e) = std::fs::remove_file(&path) {
                        warn!(path = %path.display(), error = %e, "cleanup:failed to remove prompt file");
                    } else {
                        debug!(path = %path.display(), "cleanup:prompt file removed");
                    }
                }
            }
        }

        // 2. Prune worktrees to clean up git's metadata.
        // Git will see the original path as missing since we renamed it.
        git::prune_worktrees_in(&context.git_common_dir).context("Failed to prune worktrees")?;
        debug!("cleanup:git worktrees pruned");

        // 3. Delete the local branch (unless keeping it).
        if !keep_branch {
            git::delete_branch_in(branch_name, force, &context.git_common_dir)
                .context("Failed to delete local branch")?;
            result.local_branch_deleted = true;
            info!(branch = branch_name, "cleanup:local branch deleted");
        }

        // 4. Best-effort deletion of the trash directory.
        // If the shell is inside this directory, remove_dir_all on the root might fail
        // immediately. Clearing children first ensures we reclaim the space.
        if let Some(tp) = trash_path {
            // If we're deferring window close, also defer trash deletion.
            // This prevents a race condition where processes in the window (e.g., Claude Code)
            // fail to run their stop hooks because their CWD was deleted.
            if result.window_to_close_later.is_some() {
                debug!(path = %tp.display(), "cleanup:deferring trash deletion until window close");
                result.trash_path_to_delete = Some(tp);
            } else {
                // First, aggressively clear contents to reclaim disk space
                remove_dir_contents(&tp);

                // Then try to remove the (now empty) directory
                if let Err(e) = std::fs::remove_dir(&tp) {
                    warn!(
                        path = %tp.display(),
                        error = %e,
                        "cleanup:failed to remove trash directory (likely held by active shell). \
                        The directory is empty and harmless."
                    );
                } else {
                    debug!(path = %tp.display(), "cleanup:trash directory removed");
                }
            }
        }

        Ok(())
    };

    if running_inside_target {
        let current_target = current_matching_target.unwrap();
        let target_type = if is_session_mode { "session" } else { "window" };
        info!(
            branch = branch_name,
            current_target = current_target,
            target_type,
            "cleanup:running inside matching target, deferring destructive cleanup",
        );

        // Find and kill all OTHER matching windows (not the current one)
        // Note: Sessions don't have duplicates like windows, so skip for session mode
        if mux_running && !is_session_mode {
            let matching_windows =
                find_matching_windows(context.mux.as_ref(), &context.prefix, handle)?;
            let mut killed_count = 0;
            for window in &matching_windows {
                if window != &current_target {
                    if let Err(e) = context.mux.kill_window(window) {
                        warn!(window = window, error = %e, "cleanup:failed to kill duplicate window");
                    } else {
                        killed_count += 1;
                        debug!(window = window, "cleanup:killed duplicate window");
                    }
                }
            }
            if killed_count > 0 {
                info!(
                    count = killed_count,
                    target_type, "cleanup:killed duplicate {}s", target_type
                );
            }
        }

        // Store the current window/session name for deferred close
        result.window_to_close_later = Some(current_target);

        // Run pre-remove hooks synchronously (they need the worktree intact)
        // Skip if --no-hooks is set (e.g., RPC-triggered merge).
        if worktree_path.exists()
            && !no_hooks
            && let Some(pre_remove_hooks) = &context.config.pre_remove
        {
            info!(
                branch = branch_name,
                count = pre_remove_hooks.len(),
                "cleanup:running pre-remove hooks"
            );
            let abs_worktree_path = worktree_path
                .canonicalize()
                .unwrap_or_else(|_| worktree_path.to_path_buf());
            let abs_project_root = context
                .main_worktree_root
                .canonicalize()
                .unwrap_or_else(|_| context.main_worktree_root.clone());
            let worktree_path_str = abs_worktree_path.to_string_lossy();
            let project_root_str = abs_project_root.to_string_lossy();
            let hook_env = [
                ("WORKMUX_HANDLE", handle),
                ("WM_HANDLE", handle),
                ("WM_WORKTREE_PATH", worktree_path_str.as_ref()),
                ("WM_PROJECT_ROOT", project_root_str.as_ref()),
            ];
            for command in pre_remove_hooks {
                cmd::shell_command_with_env(command, worktree_path, &hook_env)
                    .with_context(|| format!("Failed to run pre-remove command: '{}'", command))?;
            }
        }

        // Clean up prompt files immediately (harmless, doesn't affect CWD)
        let temp_dir = std::env::temp_dir();
        let prefix = format!("workmux-prompt-{}", branch_name);
        if let Ok(entries) = std::fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Some(filename) = path.file_name().and_then(|n| n.to_str())
                    && filename.starts_with(&prefix)
                    && filename.ends_with(".md")
                {
                    if let Err(e) = std::fs::remove_file(&path) {
                        warn!(path = %path.display(), error = %e, "cleanup:failed to remove prompt file");
                    } else {
                        debug!(path = %path.display(), "cleanup:prompt file removed");
                    }
                }
            }
        }

        // Defer destructive operations (rename, prune, branch delete) until after window/session close.
        // This keeps the worktree path valid so agents can run their hooks.
        if worktree_path.exists() {
            let parent = worktree_path.parent().unwrap_or_else(|| Path::new("."));
            let dir_name = worktree_path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("Invalid worktree path: no directory name"))?;
            let timestamp = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let trash_name = format!(
                ".workmux_trash_{}_{}",
                dir_name.to_string_lossy(),
                timestamp
            );
            let trash_path = parent.join(&trash_name);

            result.deferred_cleanup = Some(DeferredCleanup {
                worktree_path: worktree_path.to_path_buf(),
                trash_path,
                branch_name: branch_name.to_string(),
                keep_branch,
                force,
                git_common_dir: context.git_common_dir.clone(),
            });
            debug!(
                worktree = %worktree_path.display(),
                target_type,
                "cleanup:deferred destructive cleanup until target close",
            );
        }
    } else {
        // Not running inside any matching target, so kill it first
        if mux_running {
            if is_session_mode {
                // For session mode, kill the session directly
                let session_name = prefixed(&context.prefix, handle);
                if context.mux.session_exists(&session_name)? {
                    if let Err(e) = context.mux.kill_session(&session_name) {
                        warn!(session = session_name, error = %e, "cleanup:failed to kill session");
                    } else {
                        result.tmux_window_killed = true;
                        info!(session = session_name, "cleanup:killed session");

                        // Poll to confirm session is gone before proceeding
                        const MAX_RETRIES: u32 = 20;
                        const RETRY_DELAY: Duration = Duration::from_millis(50);
                        for _ in 0..MAX_RETRIES {
                            if !context.mux.session_exists(&session_name)? {
                                break;
                            }
                            thread::sleep(RETRY_DELAY);
                        }
                    }
                }
            } else {
                // For window mode, find and kill all matching windows (including duplicates)
                let matching_windows =
                    find_matching_windows(context.mux.as_ref(), &context.prefix, handle)?;
                let mut killed_count = 0;
                for window in &matching_windows {
                    if let Err(e) = context.mux.kill_window(window) {
                        warn!(window = window, error = %e, "cleanup:failed to kill window");
                    } else {
                        killed_count += 1;
                        debug!(window = window, "cleanup:killed window");
                    }
                }
                if killed_count > 0 {
                    result.tmux_window_killed = true;
                    info!(
                        count = killed_count,
                        handle = handle,
                        "cleanup:killed all matching windows"
                    );

                    // Poll to confirm windows are gone before proceeding
                    const MAX_RETRIES: u32 = 20;
                    const RETRY_DELAY: Duration = Duration::from_millis(50);
                    for _ in 0..MAX_RETRIES {
                        let remaining =
                            find_matching_windows(context.mux.as_ref(), &context.prefix, handle)?;
                        if remaining.is_empty() {
                            break;
                        }
                        thread::sleep(RETRY_DELAY);
                    }
                }
            }
        }
        // Now that windows/sessions are gone, clean up filesystem and git state.
        perform_fs_git_cleanup(&mut result)?;
    }

    // Clean up worktree metadata from git config
    if let Err(e) = git::remove_worktree_meta(handle) {
        warn!(handle = handle, error = %e, "cleanup:failed to remove worktree metadata");
    }

    Ok(result)
}

/// Navigate to the target branch window and close the source window.
/// Handles both cases: running inside the source window (async) and outside (sync).
/// `target_window_name` is the window name of the merge target.
/// `source_handle` is the window name of the branch being merged/removed.
pub fn navigate_to_target_and_close(
    mux: &dyn Multiplexer,
    prefix: &str,
    target_window_name: &str,
    source_handle: &str,
    cleanup_result: &CleanupResult,
    is_session_mode: bool,
) -> Result<()> {
    use crate::shell::shell_quote as shell_escape;

    /// Build the deferred cleanup script for rename, prune, branch delete, and trash removal.
    fn build_deferred_cleanup_script(dc: &DeferredCleanup) -> String {
        let wt = shell_escape(&dc.worktree_path.to_string_lossy());
        let trash = shell_escape(&dc.trash_path.to_string_lossy());
        let git_dir = shell_escape(&dc.git_common_dir.to_string_lossy());

        let mut cmds = Vec::new();
        // 1. Rename worktree to trash
        cmds.push(format!("mv {} {} >/dev/null 2>&1", wt, trash));
        // 2. Prune git worktrees
        cmds.push(format!("git -C {} worktree prune >/dev/null 2>&1", git_dir));
        // 3. Delete branch (if not keeping)
        if !dc.keep_branch {
            let branch = shell_escape(&dc.branch_name);
            let force_flag = if dc.force { "-D" } else { "-d" };
            cmds.push(format!(
                "git -C {} branch {} {} >/dev/null 2>&1",
                git_dir, force_flag, branch
            ));
        }
        // 4. Delete trash
        cmds.push(format!("rm -rf {} >/dev/null 2>&1", trash));

        format!("; {}", cmds.join("; "))
    }

    // Check if target window/session exists
    let mux_running = mux.is_running()?;
    let target_full = prefixed(prefix, target_window_name);
    let target_exists = if mux_running {
        if is_session_mode {
            mux.session_exists(&target_full)?
        } else {
            mux.window_exists(prefix, target_window_name)?
        }
    } else {
        false
    };
    let target_type = if is_session_mode { "session" } else { "window" };

    // Prepare window names for shell commands
    // Use the actual window name from window_to_close_later when available (includes -N suffix),
    // otherwise fall back to the base prefixed name
    let source_full = cleanup_result
        .window_to_close_later
        .clone()
        .unwrap_or_else(|| prefixed(prefix, source_handle));

    // Generate backend-specific shell commands for deferred scripts
    let kill_source_cmd = mux.shell_kill_window_cmd(&source_full).ok();
    let select_target_cmd = mux.shell_select_window_cmd(&target_full).ok();

    debug!(
        prefix = prefix,
        target_window_name = target_window_name,
        mux_running = mux_running,
        target_exists = target_exists,
        target_type,
        window_to_close = ?cleanup_result.window_to_close_later,
        deferred_cleanup = cleanup_result.deferred_cleanup.is_some(),
        "navigate_to_target_and_close:entry"
    );

    if !mux_running || !target_exists {
        // If target window doesn't exist, still need to close source window if running inside it
        if let Some(ref window_to_close) = cleanup_result.window_to_close_later {
            let delay = Duration::from_millis(WINDOW_CLOSE_DELAY_MS);
            let delay_secs = format!("{:.3}", delay.as_secs_f64());

            // Build cleanup script: prefer full deferred cleanup, fall back to trash-only
            let cleanup_script = if let Some(ref dc) = cleanup_result.deferred_cleanup {
                build_deferred_cleanup_script(dc)
            } else {
                cleanup_result
                    .trash_path_to_delete
                    .as_ref()
                    .map(|tp| format!("; rm -rf {}", shell_escape(&tp.to_string_lossy())))
                    .unwrap_or_default()
            };

            let kill_part = kill_source_cmd
                .as_ref()
                .map(|cmd| format!("{}; ", cmd))
                .unwrap_or_default();

            let script = format!(
                "sleep {delay}; {kill}{cleanup}",
                delay = delay_secs,
                kill = kill_part,
                cleanup = cleanup_script.strip_prefix("; ").unwrap_or(&cleanup_script),
            );
            debug!(
                script = script,
                target_type, "navigate_to_target_and_close:kill_only_script"
            );
            match mux.run_deferred_script(&script) {
                Ok(_) => info!(
                    target = window_to_close,
                    script = script,
                    target_type,
                    "cleanup:scheduled target close",
                ),
                Err(e) => warn!(
                    target = window_to_close,
                    error = ?e,
                    target_type,
                    "cleanup:failed to schedule target close",
                ),
            }
        }
        return Ok(());
    }

    if cleanup_result.window_to_close_later.is_some() {
        // Running inside a matching window: schedule navigation and kill together
        let delay = Duration::from_millis(WINDOW_CLOSE_DELAY_MS);
        let delay_secs = format!("{:.3}", delay.as_secs_f64());

        // Build cleanup script: prefer full deferred cleanup, fall back to trash-only
        let cleanup_script = if let Some(ref dc) = cleanup_result.deferred_cleanup {
            build_deferred_cleanup_script(dc)
        } else {
            cleanup_result
                .trash_path_to_delete
                .as_ref()
                .map(|tp| format!("; rm -rf {}", shell_escape(&tp.to_string_lossy())))
                .unwrap_or_default()
        };

        let select_part = select_target_cmd
            .as_ref()
            .map(|cmd| format!("{}; ", cmd))
            .unwrap_or_default();

        let kill_part = kill_source_cmd
            .as_ref()
            .map(|cmd| format!("{}; ", cmd))
            .unwrap_or_default();

        let script = format!(
            "sleep {delay}; {select}{kill}{cleanup}",
            delay = delay_secs,
            select = select_part,
            kill = kill_part,
            cleanup = cleanup_script.strip_prefix("; ").unwrap_or(&cleanup_script),
        );
        debug!(
            script = script,
            target_type, "navigate_to_target_and_close:nav_and_kill_script"
        );

        match mux.run_deferred_script(&script) {
            Ok(_) => info!(
                source = source_handle,
                target = target_window_name,
                target_type,
                "cleanup:scheduled navigation to target and source close",
            ),
            Err(e) => warn!(
                source = source_handle,
                error = ?e,
                target_type,
                "cleanup:failed to schedule navigation and source close",
            ),
        }
    } else if !cleanup_result.tmux_window_killed {
        // Running outside and targets weren't killed yet (shouldn't happen normally)
        // but handle it for completeness
        mux.select_window(prefix, target_window_name)?;
        info!(
            handle = source_handle,
            target = target_window_name,
            target_type,
            "cleanup:navigated to target branch",
        );
    }

    Ok(())
}
