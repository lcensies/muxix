use anyhow::{Context, Result};
use regex::Regex;
use std::path::Path;
use std::time::SystemTime;
use std::{thread, time::Duration};

use crate::{cmd, git, tmux};
use tracing::{debug, info, warn};

use super::context::WorkflowContext;
use super::types::CleanupResult;

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

/// Find all tmux windows matching the base handle pattern (including duplicates).
/// Matches: {prefix}{handle} and {prefix}{handle}-{N}
fn find_matching_windows(prefix: &str, handle: &str) -> Result<Vec<String>> {
    let all_windows = tmux::get_all_window_names()?;
    let base_name = tmux::prefixed(prefix, handle);
    let escaped_base = regex::escape(&base_name);
    let pattern = format!(r"^{}(-\d+)?$", escaped_base);
    let re = Regex::new(&pattern).expect("Invalid regex pattern");

    let matching: Vec<String> = all_windows.into_iter().filter(|w| re.is_match(w)).collect();

    Ok(matching)
}

/// Check if the current window matches the base handle pattern (including duplicates).
fn is_inside_matching_window(prefix: &str, handle: &str) -> Result<Option<String>> {
    let current_window = match tmux::current_window_name()? {
        Some(name) => name,
        None => return Ok(None),
    };

    let base_name = tmux::prefixed(prefix, handle);
    let escaped_base = regex::escape(&base_name);
    let pattern = format!(r"^{}(-\d+)?$", escaped_base);
    let re = Regex::new(&pattern).expect("Invalid regex pattern");

    if re.is_match(&current_window) {
        Ok(Some(current_window))
    } else {
        Ok(None)
    }
}

/// Centralized function to clean up tmux and git resources.
/// `branch_name` is used for git operations (branch deletion).
/// `handle` is used for tmux operations (window lookup/kill).
pub fn cleanup(
    context: &WorkflowContext,
    branch_name: &str,
    handle: &str,
    worktree_path: &Path,
    force: bool,
    keep_branch: bool,
) -> Result<CleanupResult> {
    info!(
        branch = branch_name,
        handle = handle,
        path = %worktree_path.display(),
        force,
        keep_branch,
        "cleanup:start"
    );
    // Change the CWD to main worktree before any destructive operations.
    // This prevents "Unable to read current working directory" errors when the command
    // is run from within the worktree being deleted.
    context.chdir_to_main_worktree()?;

    let tmux_running = tmux::is_running().unwrap_or(false);

    // Check if we're running inside ANY matching window (original or duplicate)
    let current_matching_window = if tmux_running {
        is_inside_matching_window(&context.prefix, handle)?
    } else {
        None
    };
    let running_inside_target_window = current_matching_window.is_some();

    let mut result = CleanupResult {
        tmux_window_killed: false,
        worktree_removed: false,
        local_branch_deleted: false,
        window_to_close_later: None,
        trash_path_to_delete: None,
    };

    // Helper closure to perform the actual filesystem and git cleanup.
    // This avoids code duplication while enforcing the correct operational order.
    let perform_fs_git_cleanup = |result: &mut CleanupResult| -> Result<()> {
        // Run pre-remove hooks before removing the worktree directory.
        // Skip if the worktree directory doesn't exist (e.g., user manually deleted it).
        if worktree_path.exists() {
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

    if running_inside_target_window {
        let current_window = current_matching_window.unwrap();
        info!(
            branch = branch_name,
            current_window = current_window,
            "cleanup:running inside matching window, will close all others first"
        );

        // Find and kill all OTHER matching windows (not the current one)
        if tmux_running {
            let matching_windows = find_matching_windows(&context.prefix, handle)?;
            let mut killed_count = 0;
            for window in &matching_windows {
                if window != &current_window {
                    if let Err(e) = tmux::kill_window_by_full_name(window) {
                        warn!(window = window, error = %e, "cleanup:failed to kill duplicate window");
                    } else {
                        killed_count += 1;
                        debug!(window = window, "cleanup:killed duplicate window");
                    }
                }
            }
            if killed_count > 0 {
                info!(count = killed_count, "cleanup:killed duplicate windows");
            }
        }

        // Store the current window name for deferred close
        result.window_to_close_later = Some(current_window);

        // Perform filesystem and git cleanup
        perform_fs_git_cleanup(&mut result)?;
    } else {
        // Not running inside any matching window, so kill ALL matching windows first
        if tmux_running {
            let matching_windows = find_matching_windows(&context.prefix, handle)?;
            let mut killed_count = 0;
            for window in &matching_windows {
                if let Err(e) = tmux::kill_window_by_full_name(window) {
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
                    let remaining = find_matching_windows(&context.prefix, handle)?;
                    if remaining.is_empty() {
                        break;
                    }
                    thread::sleep(RETRY_DELAY);
                }
            }
        }
        // Now that windows are gone, clean up filesystem and git state.
        perform_fs_git_cleanup(&mut result)?;
    }

    Ok(result)
}

/// Navigate to the target branch window and close the source window.
/// Handles both cases: running inside the source window (async) and outside (sync).
/// `target_window_name` is the tmux window name of the merge target.
/// `source_handle` is the tmux window name of the branch being merged/removed.
pub fn navigate_to_target_and_close(
    prefix: &str,
    target_window_name: &str,
    source_handle: &str,
    cleanup_result: &CleanupResult,
) -> Result<()> {
    /// Helper function to shell-escape strings for safe inclusion in shell commands
    fn shell_escape(s: &str) -> String {
        format!("'{}'", s.replace('\'', r#"'\''"#))
    }

    // Check if target window exists
    let tmux_running = tmux::is_running()?;
    let target_exists = if tmux_running {
        tmux::window_exists(prefix, target_window_name)?
    } else {
        false
    };
    debug!(
        prefix = prefix,
        target_window_name = target_window_name,
        tmux_running = tmux_running,
        target_exists = target_exists,
        window_to_close = ?cleanup_result.window_to_close_later,
        "navigate_to_target_and_close:entry"
    );
    if !tmux_running || !target_exists {
        // If target window doesn't exist, still need to close source window if running inside it
        if let Some(ref window_to_close) = cleanup_result.window_to_close_later {
            let delay = Duration::from_millis(WINDOW_CLOSE_DELAY_MS);
            let delay_secs = format!("{:.3}", delay.as_secs_f64());
            let source_spec = format!("={}", window_to_close);
            let source_escaped = shell_escape(&source_spec);

            // Append trash deletion if deferred
            let trash_removal = cleanup_result
                .trash_path_to_delete
                .as_ref()
                .map(|tp| format!("; rm -rf {}", shell_escape(&tp.to_string_lossy())))
                .unwrap_or_default();

            let script = format!(
                "sleep {delay}; tmux kill-window -t {source} >/dev/null 2>&1{trash_removal}",
                delay = delay_secs,
                source = source_escaped,
                trash_removal = trash_removal,
            );
            debug!(
                script = script,
                "navigate_to_target_and_close:kill_only_script"
            );
            match tmux::run_shell(&script) {
                Ok(_) => info!(
                    window = window_to_close,
                    script = script,
                    "cleanup:scheduled window close"
                ),
                Err(e) => warn!(
                    window = window_to_close,
                    error = ?e,
                    "cleanup:failed to schedule window close",
                ),
            }
        }
        return Ok(());
    }

    if let Some(ref window_to_close) = cleanup_result.window_to_close_later {
        // Running inside a matching window: schedule both navigation and kill together
        let delay = Duration::from_millis(WINDOW_CLOSE_DELAY_MS);
        let delay_secs = format!("{:.3}", delay.as_secs_f64());
        let target_spec = format!("={}", tmux::prefixed(prefix, target_window_name));
        let source_spec = format!("={}", window_to_close);
        let target_escaped = shell_escape(&target_spec);
        let source_escaped = shell_escape(&source_spec);

        // Append trash deletion if deferred
        let trash_removal = cleanup_result
            .trash_path_to_delete
            .as_ref()
            .map(|tp| format!("; rm -rf {}", shell_escape(&tp.to_string_lossy())))
            .unwrap_or_default();

        let script = format!(
            "sleep {delay}; tmux select-window -t {target} >/dev/null 2>&1; tmux kill-window -t {source} >/dev/null 2>&1{trash_removal}",
            delay = delay_secs,
            target = target_escaped,
            source = source_escaped,
            trash_removal = trash_removal,
        );
        debug!(
            script = script,
            "navigate_to_target_and_close:nav_and_kill_script"
        );

        match tmux::run_shell(&script) {
            Ok(_) => info!(
                window = window_to_close,
                target = target_window_name,
                "cleanup:scheduled navigation to target and window close"
            ),
            Err(e) => warn!(
                window = window_to_close,
                error = ?e,
                "cleanup:failed to schedule navigation and window close",
            ),
        }
    } else if !cleanup_result.tmux_window_killed {
        // Running outside and windows weren't killed yet (shouldn't happen normally)
        // but handle it for completeness
        tmux::select_window(prefix, target_window_name)?;
        info!(
            handle = source_handle,
            target = target_window_name,
            "cleanup:navigated to target branch window"
        );
    }

    Ok(())
}
