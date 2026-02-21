use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

use crate::cmd::Cmd;
use crate::config::MuxMode;

use super::WorktreeNotFound;
use super::branch::unset_branch_upstream;

/// Check if a worktree already exists for a branch
pub fn worktree_exists(branch_name: &str) -> Result<bool> {
    match get_worktree_path(branch_name) {
        Ok(_) => Ok(true),
        Err(e) => {
            // Check if this is a WorktreeNotFound error
            if e.is::<WorktreeNotFound>() {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

/// Create a new git worktree
pub fn create_worktree(
    worktree_path: &Path,
    branch_name: &str,
    create_branch: bool,
    base_branch: Option<&str>,
    track_upstream: bool,
) -> Result<()> {
    let path_str = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("Invalid worktree path"))?;

    let mut cmd = Cmd::new("git").arg("worktree").arg("add");

    if create_branch {
        cmd = cmd.arg("-b").arg(branch_name).arg(path_str);
        if let Some(base) = base_branch {
            cmd = cmd.arg(base);
        }
    } else {
        cmd = cmd.arg(path_str).arg(branch_name);
    }

    cmd.run().context("Failed to create worktree")?;

    // When creating a new branch from a remote tracking branch (e.g., origin/main),
    // git automatically sets up tracking for the new branch. This is desirable when
    // opening a remote branch locally, but we unset the upstream when the new branch
    // should be independent.
    if create_branch && !track_upstream {
        unset_branch_upstream(branch_name)?;
    }

    Ok(())
}

/// Prune stale worktree metadata.
pub fn prune_worktrees_in(git_common_dir: &Path) -> Result<()> {
    Cmd::new("git")
        .workdir(git_common_dir)
        .args(&["worktree", "prune"])
        .run()
        .context("Failed to prune worktrees")?;
    Ok(())
}

/// Parse the output of `git worktree list --porcelain`
pub(super) fn parse_worktree_list_porcelain(output: &str) -> Result<Vec<(PathBuf, String)>> {
    let mut worktrees = Vec::new();
    for block in output.trim().split("\n\n") {
        let mut path: Option<PathBuf> = None;
        let mut branch: Option<String> = None;

        for line in block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
                branch = Some(b.to_string());
            } else if line.trim() == "detached" {
                branch = Some("(detached)".to_string());
            }
        }

        if let (Some(p), Some(b)) = (path, branch) {
            worktrees.push((p, b));
        }
    }
    Ok(worktrees)
}

/// Get the path to a worktree for a given branch
pub fn get_worktree_path(branch_name: &str) -> Result<PathBuf> {
    let list_str = Cmd::new("git")
        .args(&["worktree", "list", "--porcelain"])
        .run_and_capture_stdout()
        .context("Failed to list worktrees while locating worktree path")?;

    let worktrees = parse_worktree_list_porcelain(&list_str)?;

    for (path, branch) in worktrees {
        if branch == branch_name {
            return Ok(path);
        }
    }

    Err(WorktreeNotFound(branch_name.to_string()).into())
}

/// Find a worktree by handle (directory name) or branch name.
/// Tries handle first, then falls back to branch lookup.
/// Returns both the path and the branch name checked out in that worktree.
pub fn find_worktree(name: &str) -> Result<(PathBuf, String)> {
    let list_str = Cmd::new("git")
        .args(&["worktree", "list", "--porcelain"])
        .run_and_capture_stdout()
        .context("Failed to list worktrees")?;

    let worktrees = parse_worktree_list_porcelain(&list_str)?;

    // First: try to match by handle (directory name)
    for (path, branch) in &worktrees {
        if let Some(dir_name) = path.file_name()
            && dir_name.to_string_lossy() == name
        {
            return Ok((path.clone(), branch.clone()));
        }
    }

    // Fallback: try to match by branch name
    for (path, branch) in worktrees {
        if branch == name {
            return Ok((path, branch));
        }
    }

    Err(WorktreeNotFound(name.to_string()).into())
}

/// List all worktrees with their branches
pub fn list_worktrees() -> Result<Vec<(PathBuf, String)>> {
    let list = Cmd::new("git")
        .args(&["worktree", "list", "--porcelain"])
        .run_and_capture_stdout()
        .context("Failed to list worktrees")?;
    parse_worktree_list_porcelain(&list)
}

/// Store per-worktree metadata in git config.
pub fn set_worktree_meta(handle: &str, key: &str, value: &str) -> Result<()> {
    Cmd::new("git")
        .args(&[
            "config",
            "--local",
            &format!("workmux.worktree.{}.{}", handle, key),
            value,
        ])
        .run()
        .with_context(|| format!("Failed to set worktree metadata {}.{}", handle, key))?;
    Ok(())
}

/// Retrieve per-worktree metadata from git config.
/// Returns None if the key doesn't exist.
pub fn get_worktree_meta(handle: &str, key: &str) -> Option<String> {
    Cmd::new("git")
        .args(&[
            "config",
            "--local",
            "--get",
            &format!("workmux.worktree.{}.{}", handle, key),
        ])
        .run_and_capture_stdout()
        .ok()
        .filter(|s| !s.is_empty())
}

/// Determine the tmux mode for a worktree from git metadata.
/// Falls back to Window mode if no metadata is found (backward compatibility).
pub fn get_worktree_mode(handle: &str) -> MuxMode {
    match get_worktree_meta(handle, "mode") {
        Some(mode) if mode == "session" => MuxMode::Session,
        _ => MuxMode::Window,
    }
}

/// Remove all metadata for a worktree handle.
pub fn remove_worktree_meta(handle: &str) -> Result<()> {
    // Use --remove-section to remove all keys under the handle's section
    let _ = Cmd::new("git")
        .args(&[
            "config",
            "--local",
            "--remove-section",
            &format!("workmux.worktree.{}", handle),
        ])
        .run();
    Ok(())
}

/// Get the main worktree root directory (not a linked worktree)
///
/// For bare repositories with linked worktrees, this returns the bare repo path.
/// For regular repositories, this returns the first worktree that exists on disk.
pub fn get_main_worktree_root() -> Result<PathBuf> {
    let list_str = Cmd::new("git")
        .args(&["worktree", "list", "--porcelain"])
        .run_and_capture_stdout()
        .context("Failed to list worktrees while locating main worktree")?;

    // Check if this is a bare repo setup.
    // The first entry in `git worktree list` is always the main worktree or bare repo.
    // For bare repos, it looks like:
    //   worktree /path/to/.bare
    //   bare
    if let Some(first_block) = list_str.trim().split("\n\n").next() {
        let mut path: Option<PathBuf> = None;
        let mut is_bare = false;

        for line in first_block.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                path = Some(PathBuf::from(p));
            } else if line.trim() == "bare" {
                is_bare = true;
            }
        }

        // If this is a bare repo, return its path immediately.
        // Git commands like `git worktree prune` work correctly from bare repo directories.
        if is_bare && let Some(p) = path {
            return Ok(p);
        }
    }

    // Not a bare repo - find the first worktree that exists on disk.
    // This handles edge cases where a worktree was deleted but not yet pruned.
    let worktrees = parse_worktree_list_porcelain(&list_str)?;

    for (path, _) in &worktrees {
        if path.exists() {
            return Ok(path.clone());
        }
    }

    // Fallback: return the first worktree even if it doesn't exist
    if let Some((path, _)) = worktrees.first() {
        Ok(path.clone())
    } else {
        Err(anyhow!("No main worktree found"))
    }
}
