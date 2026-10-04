use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;

use crate::{cmd, git};
use tracing::{debug, info};

use super::cleanup;
use super::context::WorkflowContext;
use super::types::MergeResult;

/// Outcome of an attempted merge.
///
/// Distinguishes a clean merge from a *conflict* — a recoverable state that a
/// human or an agent can resolve — so callers (notably the orchestrator) can
/// react differently to each without parsing error strings.
#[derive(Debug)]
pub enum MergeOutcome {
    /// The branch merged cleanly into the target.
    Merged(MergeResult),
    /// The merge or rebase hit conflicts. The feature worktree is left in a
    /// state ready for resolution (or aborted clean, see `merge_with_outcome`).
    Conflict {
        worktree_path: PathBuf,
        target_branch: String,
        branch: String,
    },
}

/// Merge a branch into the target branch and clean up.
///
/// Conflicts are surfaced as an `Err` with resolution guidance, preserving the
/// historical CLI behaviour (the in-progress rebase/merge is left for the user
/// to resolve manually).
#[allow(clippy::too_many_arguments)]
pub fn merge(
    name: &str,
    into_branch: Option<&str>,
    ignore_uncommitted: bool,
    rebase: bool,
    squash: bool,
    keep: bool,
    no_verify: bool,
    no_hooks: bool,
    notification: bool,
    context: &WorkflowContext,
) -> Result<MergeResult> {
    match merge_inner(
        name,
        into_branch,
        ignore_uncommitted,
        rebase,
        squash,
        keep,
        no_verify,
        no_hooks,
        notification,
        false, // leave conflicted state for manual resolution
        context,
    )? {
        MergeOutcome::Merged(result) => Ok(result),
        MergeOutcome::Conflict {
            worktree_path,
            target_branch,
            branch,
        } => {
            let retry_cmd = if into_branch.is_some() {
                format!("muxix merge {} --into {}", branch, target_branch)
            } else {
                format!("muxix merge {}", branch)
            };
            Err(anyhow!(
                "Merge failed due to conflicts. Target worktree kept clean.\n\n\
                To resolve, update your branch in worktree at {}:\n\
                  git rebase {}  (recommended)\n\
                Or:\n\
                  git merge {}\n\n\
                After resolving conflicts, retry: {}",
                worktree_path.display(),
                target_branch,
                target_branch,
                retry_cmd
            ))
        }
    }
}

/// Merge a branch into the target, returning a typed [`MergeOutcome`] instead of
/// collapsing conflicts into an error.
///
/// On conflict the in-progress rebase is aborted so the feature worktree is left
/// clean, ready to be handed to an agent for a fresh resolution attempt. Used by
/// the orchestrator's deterministic merge step.
#[allow(clippy::too_many_arguments)]
#[allow(dead_code)]
pub fn merge_with_outcome(
    name: &str,
    into_branch: Option<&str>,
    ignore_uncommitted: bool,
    rebase: bool,
    squash: bool,
    keep: bool,
    no_verify: bool,
    no_hooks: bool,
    notification: bool,
    context: &WorkflowContext,
) -> Result<MergeOutcome> {
    merge_inner(
        name,
        into_branch,
        ignore_uncommitted,
        rebase,
        squash,
        keep,
        no_verify,
        no_hooks,
        notification,
        true, // abort conflicted rebase for a clean agent handoff
        context,
    )
}

/// Shared merge implementation. `abort_on_conflict` controls whether an
/// in-progress rebase is aborted when a conflict is detected (clean handoff) or
/// left in place for manual resolution.
#[allow(clippy::too_many_arguments)]
fn merge_inner(
    name: &str,
    into_branch: Option<&str>,
    ignore_uncommitted: bool,
    rebase: bool,
    squash: bool,
    keep: bool,
    no_verify: bool,
    no_hooks: bool,
    notification: bool,
    abort_on_conflict: bool,
    context: &WorkflowContext,
) -> Result<MergeOutcome> {
    info!(
        name = name,
        into = into_branch,
        ignore_uncommitted,
        rebase,
        squash,
        keep,
        no_verify,
        no_hooks,
        "merge:start"
    );

    // Every git lookup below runs in the main worktree rather than the process
    // CWD: the CWD may be inside the worktree about to be deleted, and the
    // daemon runs several projects' merges from one process, so changing it
    // globally is not an option.
    let repo = context.main_worktree_root.as_path();

    // Smart resolution: try handle first, then branch name
    let (worktree_path, branch_to_merge) =
        git::find_worktree_in(name, Some(repo)).map_err(|_| {
            anyhow!(
                "Worktree '{}' not found. Use 'muxix list' to see available worktrees.",
                name
            )
        })?;

    // The handle is the basename of the worktree directory (used for tmux operations)
    let handle = worktree_path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| {
            anyhow!(
                "Could not derive handle from worktree path: {}",
                worktree_path.display()
            )
        })?;

    // Capture mode BEFORE cleanup (cleanup removes the metadata)
    let mode = git::get_worktree_mode_in(handle, Some(repo));

    debug!(
        name = name,
        handle = handle,
        branch = branch_to_merge,
        path = %worktree_path.display(),
        "merge:worktree resolved"
    );

    // Determine the target branch:
    // 1. Use explicit --into if provided
    // 2. Otherwise, check if branch has a stored base (from muxix add)
    // 3. Fall back to main_branch
    let detected_base: Option<String> = if into_branch.is_some() {
        None // User explicitly specified target, no auto-detection needed
    } else {
        match git::get_branch_base_in(&branch_to_merge, Some(repo)) {
            Ok(base) => {
                // Verify the base branch still exists locally.
                if git::local_branch_exists_in(&base, Some(repo))? {
                    info!(
                        branch = %branch_to_merge,
                        base = %base,
                        "merge:auto-detected base branch"
                    );
                    Some(base)
                } else {
                    info!(
                        branch = %branch_to_merge,
                        base = %base,
                        "merge:base branch not found locally, defaulting to main"
                    );
                    None
                }
            }
            Err(_) => {
                debug!(
                    branch = %branch_to_merge,
                    "merge:no base config found, defaulting to main"
                );
                None
            }
        }
    };

    let target_branch = into_branch
        .map(|s| s.to_string())
        .or(detected_base)
        .unwrap_or_else(|| context.main_branch.clone());
    let target_branch = target_branch.as_str();

    // Resolve the worktree path and window handle for the TARGET branch.
    // We prioritize finding an existing worktree for the target branch to support
    // workflows where 'main' is checked out in a linked worktree (issue #29).
    let (target_worktree_path, target_window_name) =
        match git::get_worktree_path_in(target_branch, Some(repo)) {
            Ok(path) => {
                // Target is checked out in a worktree (could be main root or a linked worktree)
                if path == context.main_worktree_root {
                    // It's in the main root. Use the main branch name as the window handle.
                    (path, context.main_branch.clone())
                } else {
                    // It's in a linked worktree. Use the directory name as the handle.
                    let handle = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .ok_or_else(|| anyhow!("Invalid worktree path for target branch"))?
                        .to_string();
                    (path, handle)
                }
            }
            Err(_) => {
                // Target branch is NOT checked out anywhere.
                // We fallback to using the main worktree root to perform the merge.
                debug!(
                    target = target_branch,
                    "merge:target branch has no worktree, using main worktree"
                );
                (
                    context.main_worktree_root.clone(),
                    context.main_branch.clone(),
                )
            }
        };

    // Handle changes in the source worktree
    // Only check for unstaged/untracked when worktree will be deleted (!keep)
    // With --keep, the worktree persists so no data loss risk
    let has_unstaged = !keep && git::has_unstaged_changes(&worktree_path)?;
    let has_untracked = !keep && git::has_untracked_files(&worktree_path)?;

    if (has_unstaged || has_untracked) && !ignore_uncommitted {
        let mut issues = Vec::new();
        if has_unstaged {
            issues.push("unstaged changes");
        }
        if has_untracked {
            issues.push("untracked files (will be lost)");
        }
        return Err(anyhow!(
            "Worktree for '{}' has {}. Please stage or stash them, or use --ignore-uncommitted.",
            branch_to_merge,
            issues.join(" and ")
        ));
    }

    let had_staged_changes = git::has_staged_changes(&worktree_path)?;
    if had_staged_changes && !ignore_uncommitted {
        // Commit using git's editor (respects $EDITOR or git config)
        info!(path = %worktree_path.display(), "merge:committing staged changes");
        git::commit_with_editor(&worktree_path).context("Failed to commit staged changes")?;
    }

    if branch_to_merge == target_branch {
        return Err(anyhow!(
            "Cannot merge branch '{}' into itself.",
            branch_to_merge
        ));
    }
    debug!(
        branch = %branch_to_merge,
        target = target_branch,
        "merge:target branch resolved"
    );

    // Safety check: Abort if the target worktree has uncommitted tracked changes.
    // Untracked files are allowed; git will fail safely if they collide with merged files.
    if git::has_tracked_changes(&target_worktree_path)? {
        return Err(anyhow!(
            "Target worktree ({}) has uncommitted changes. Please commit or stash them before merging.",
            target_worktree_path.display()
        ));
    }

    // Explicitly switch the target worktree to the target branch.
    // This ensures that if we are reusing the main worktree for a feature branch merge,
    // it is checked out to the correct branch.
    git::switch_branch_in_worktree(&target_worktree_path, target_branch)?;

    // Run pre-merge hooks after all validations pass but before any merge operations begin.
    // Skip hooks if --no-verify or --no-hooks flag is passed.
    if !no_verify
        && !no_hooks
        && let Some(hooks) = &context.config.pre_merge
        && !hooks.is_empty()
    {
        info!(count = hooks.len(), "merge:running pre-merge hooks");

        let abs_worktree_path = worktree_path
            .canonicalize()
            .unwrap_or_else(|_| worktree_path.clone());
        let abs_project_root = context
            .main_worktree_root
            .canonicalize()
            .unwrap_or_else(|_| context.main_worktree_root.clone());
        let worktree_path_str = abs_worktree_path.to_string_lossy();
        let project_root_str = abs_project_root.to_string_lossy();

        let hook_env = [
            ("MUXIX_HANDLE", handle),
            ("MUXIX_BRANCH_NAME", branch_to_merge.as_str()),
            ("MUXIX_TARGET_BRANCH", target_branch),
            ("MUXIX_WORKTREE_PATH", worktree_path_str.as_ref()),
            ("MUXIX_PROJECT_ROOT", project_root_str.as_ref()),
        ];

        for command in hooks {
            cmd::shell_command_with_env(command, &worktree_path, &hook_env)
                .with_context(|| format!("Pre-merge hook failed: '{}'", command))?;
        }
    }

    // Merge submodule worktrees first, then update the parent's submodule pointer.
    if context.config.submodules.worktrees {
        let sub_worktrees =
            git::get_submodule_worktrees(&context.main_worktree_root, &worktree_path)
                .unwrap_or_default();

        for (submodule, _sub_wt_path) in &sub_worktrees {
            info!(
                submodule = submodule.name,
                branch = %branch_to_merge,
                target = target_branch,
                "merge:merging submodule worktree"
            );
            match git::merge_submodule_worktree(
                &context.main_worktree_root,
                submodule,
                &branch_to_merge,
                target_branch,
            ) {
                Ok(()) => {
                    git::stage_path_in_worktree(&target_worktree_path, &submodule.path)
                        .with_context(|| {
                            format!(
                                "Failed to stage submodule pointer for '{}' after merge",
                                submodule.name
                            )
                        })?;
                    info!(submodule = submodule.name, "merge:submodule pointer staged");
                }
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!(
                            "Submodule '{}' merge failed. Resolve conflicts in {} first.",
                            submodule.name,
                            context.main_worktree_root.join(&submodule.path).display()
                        )
                    });
                }
            }
        }

        if !sub_worktrees.is_empty() && git::has_staged_changes(&target_worktree_path)? {
            git::commit_with_message(
                &target_worktree_path,
                &format!("chore: update submodule pointers for '{}'", branch_to_merge),
            )
            .context("Failed to commit updated submodule pointers")?;
            info!("merge:submodule pointer commit created");
        }
    }

    // Helper to build a Conflict outcome for the current merge target.
    let conflict_outcome = |branch: &str| -> MergeOutcome {
        MergeOutcome::Conflict {
            worktree_path: worktree_path.clone(),
            target_branch: target_branch.to_string(),
            branch: branch.to_string(),
        }
    };

    if rebase {
        // Rebase the feature branch on top of target inside its own worktree.
        // This is where conflicts will be detected.
        println!("Rebasing '{}' onto '{}'...", branch_to_merge, target_branch);
        info!(
            branch = %branch_to_merge,
            base = target_branch,
            "merge:rebase start"
        );
        if let Err(e) = git::rebase_branch_onto_base(&worktree_path, target_branch) {
            // A rebase failure is usually a conflict; confirm via worktree state
            // so genuine errors (bad ref, etc.) still surface as errors.
            if git::worktree_has_conflicts(&worktree_path).unwrap_or(false) {
                info!(branch = %branch_to_merge, "merge:rebase conflict");
                if abort_on_conflict {
                    let _ = git::abort_rebase_in_worktree(&worktree_path);
                }
                return Ok(conflict_outcome(&branch_to_merge));
            }
            return Err(e).with_context(|| {
                format!(
                    "Rebase failed.\n\n\
                    Please resolve manually inside the worktree at '{}'.\n\
                    Then, run 'git rebase --continue' to proceed or 'git rebase --abort' to cancel.",
                    worktree_path.display()
                )
            });
        }

        // After a successful rebase, merge into target. This will be a fast-forward.
        git::merge_in_worktree(&target_worktree_path, &branch_to_merge)
            .context("Failed to merge rebased branch. This should have been a fast-forward.")?;
        info!(branch = %branch_to_merge, "merge:fast-forward complete");
    } else if squash {
        // Perform the squash merge. This stages all changes from the feature branch but does not commit.
        if let Err(e) = git::merge_squash_in_worktree(&target_worktree_path, &branch_to_merge) {
            info!(branch = %branch_to_merge, error = %e, "merge:squash merge failed, resetting target worktree");
            // Best effort to reset; ignore failure as the user message is the priority.
            let _ = git::reset_hard(&target_worktree_path);
            return Ok(conflict_outcome(&branch_to_merge));
        }

        // Prompt the user to provide a commit message for the squashed changes.
        println!("Staged squashed changes. Please provide a commit message in your editor.");
        git::commit_with_editor(&target_worktree_path)
            .context("Failed to commit squashed changes. You may need to commit them manually.")?;
        info!(branch = %branch_to_merge, "merge:squash merge committed");
    } else {
        // Default merge commit workflow
        if let Err(e) = git::merge_in_worktree(&target_worktree_path, &branch_to_merge) {
            info!(branch = %branch_to_merge, error = %e, "merge:standard merge failed, aborting merge in target worktree");
            // Best effort to abort; ignore failure as the user message is the priority.
            let _ = git::abort_merge_in_worktree(&target_worktree_path);
            return Ok(conflict_outcome(&branch_to_merge));
        }
        info!(branch = %branch_to_merge, "merge:standard merge complete");
    }

    // Show notification before cleanup or early return (--keep),
    // since cleanup may kill the window and terminate this process
    if notification {
        show_notification(&format!(
            "Merged '{}' into '{}'",
            branch_to_merge, target_branch
        ));
    }

    // Skip cleanup when keep behavior is enabled
    if keep {
        info!(branch = %branch_to_merge, "merge:skipping cleanup");
        return Ok(MergeOutcome::Merged(MergeResult {
            branch_merged: branch_to_merge,
            main_branch: target_branch.to_string(),
            had_staged_changes,
        }));
    }

    // Always force cleanup after a successful merge
    info!(branch = %branch_to_merge, "merge:cleanup start");
    let cleanup_result = cleanup::cleanup(
        context,
        &branch_to_merge,
        handle,
        &worktree_path,
        true,
        false, // keep_branch: always delete when merging
        no_hooks,
    )?;

    // Navigate to the target branch window/session and close the source
    cleanup::navigate_to_target_and_close(
        context.mux.as_ref(),
        &context.prefix,
        &target_window_name,
        handle,
        &cleanup_result,
        mode,
    )?;

    Ok(MergeOutcome::Merged(MergeResult {
        branch_merged: branch_to_merge,
        main_branch: target_branch.to_string(),
        had_staged_changes,
    }))
}

/// Shows a system notification on macOS or Linux
fn show_notification(message: &str) {
    #[cfg(target_os = "macos")]
    {
        use mac_notification_sys::{Notification, set_application};
        // Set application to Terminal to use its icon
        if let Err(e) = set_application("com.apple.Terminal") {
            tracing::debug!("Failed to set notification application: {:?}", e);
        }
        if let Err(e) = Notification::default()
            .title("muxix")
            .message(message)
            .send()
        {
            tracing::debug!("Failed to send notification: {:?}", e);
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        if let Err(e) = notify_rust::Notification::new()
            .summary("muxix")
            .body(message)
            .show()
        {
            tracing::debug!("Failed to send notification: {:?}", e);
        }
    }
}
