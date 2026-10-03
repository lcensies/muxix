use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::process::Command;

use crate::cmd::Cmd;

/// Commit staged changes in a worktree using the user's editor
pub fn commit_with_editor(worktree_path: &Path) -> Result<()> {
    let status = Command::new("git")
        .current_dir(worktree_path)
        .arg("commit")
        .status()
        .context("Failed to run git commit")?;

    if !status.success() {
        return Err(anyhow!("Commit was aborted or failed"));
    }

    Ok(())
}

/// Merge a branch into the current branch in a specific worktree
pub fn merge_in_worktree(worktree_path: &Path, branch_name: &str) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["merge", branch_name])
        .run()
        .context("Failed to merge")?;
    Ok(())
}

/// Rebase the current branch in a worktree onto a base branch
pub fn rebase_branch_onto_base(worktree_path: &Path, base_branch: &str) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["rebase", base_branch])
        .run()
        .with_context(|| format!("Failed to rebase onto '{}'", base_branch))?;
    Ok(())
}

/// Perform a squash merge in a specific worktree (does not commit)
pub fn merge_squash_in_worktree(worktree_path: &Path, branch_name: &str) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["merge", "--squash", branch_name])
        .run()
        .context("Failed to perform squash merge")?;
    Ok(())
}

/// Switch to a different branch in a specific worktree
pub fn switch_branch_in_worktree(worktree_path: &Path, branch_name: &str) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["switch", branch_name])
        .run()
        .with_context(|| {
            format!(
                "Failed to switch to branch '{}' in worktree '{}'",
                branch_name,
                worktree_path.display()
            )
        })?;
    Ok(())
}

/// Stash uncommitted changes, optionally including untracked files or using patch mode.
pub fn stash_push(message: &str, include_untracked: bool, patch: bool) -> Result<()> {
    if patch {
        // For --patch mode, we need an interactive terminal
        let status = Command::new("git")
            .args(["stash", "push", "-m", message, "--patch"])
            .status()
            .context("Failed to run interactive git stash")?;

        if !status.success() {
            return Err(anyhow!(
                "Git stash --patch failed. Make sure you select at least one hunk."
            ));
        }
    } else {
        let mut cmd = Cmd::new("git").args(&["stash", "push", "-m", message]);

        if include_untracked {
            cmd = cmd.arg("--include-untracked");
        }

        cmd.run().context("Failed to stash changes")?;
    }
    Ok(())
}

/// Pop the latest stash in a specific worktree.
pub fn stash_pop(worktree_path: &Path) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["stash", "pop"])
        .run()
        .context("Failed to apply stashed changes. Conflicts may have occurred.")?;
    Ok(())
}

/// Reset the worktree to HEAD, discarding all local changes.
pub fn reset_hard(worktree_path: &Path) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["reset", "--hard", "HEAD"])
        .run()
        .context("Failed to reset worktree")?;
    Ok(())
}

/// Abort a merge in progress in a specific worktree
pub fn abort_merge_in_worktree(worktree_path: &Path) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["merge", "--abort"])
        .run()
        .context("Failed to abort merge. The worktree may not be in a merging state.")?;
    Ok(())
}

/// Abort a rebase in progress in a specific worktree.
pub fn abort_rebase_in_worktree(worktree_path: &Path) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["rebase", "--abort"])
        .run()
        .context("Failed to abort rebase. The worktree may not be in a rebasing state.")?;
    Ok(())
}

/// Returns true if the worktree currently has unmerged (conflicted) paths or an
/// in-progress merge/rebase. Used to distinguish a merge *conflict* — which a
/// human or agent can resolve — from any other kind of failure.
///
/// `git ls-files --unmerged` lists entries with a non-zero stage (the hallmark
/// of a conflict). We also treat an in-progress rebase/merge as conflicted so
/// the caller can hand the half-applied state off for resolution.
pub fn worktree_has_conflicts(worktree_path: &Path) -> Result<bool> {
    let out = Command::new("git")
        .current_dir(worktree_path)
        .args(["ls-files", "--unmerged"])
        .output()
        .context("Failed to query unmerged files")?;
    if !out.stdout.is_empty() {
        return Ok(true);
    }

    // A rebase or merge left in progress also signals an unresolved conflict.
    let git_dir = Command::new("git")
        .current_dir(worktree_path)
        .args(["rev-parse", "--git-path", "."])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| worktree_path.join(".git"));

    let in_progress = git_dir.join("MERGE_HEAD").exists()
        || git_dir.join("rebase-merge").exists()
        || git_dir.join("rebase-apply").exists();
    Ok(in_progress)
}

/// Stage a specific path (file or directory) in a worktree.
pub fn stage_path_in_worktree(worktree_path: &Path, path: &str) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["add", path])
        .run()
        .with_context(|| {
            format!(
                "Failed to stage '{}' in worktree '{}'",
                path,
                worktree_path.display()
            )
        })?;
    Ok(())
}

/// Commit staged changes in a worktree with a fixed message (non-interactive).
pub fn commit_with_message(worktree_path: &Path, message: &str) -> Result<()> {
    Cmd::new("git")
        .workdir(worktree_path)
        .args(&["commit", "-m", message])
        .run()
        .context("Failed to commit with message")?;
    Ok(())
}
