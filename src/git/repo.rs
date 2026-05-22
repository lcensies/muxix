use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

use crate::cmd::Cmd;

/// Check if a path is ignored by git (via .gitignore, global gitignore, etc.)
pub fn is_path_ignored(repo_path: &Path, file_path: &str) -> bool {
    std::process::Command::new("git")
        .args(["check-ignore", "-q", file_path])
        .current_dir(repo_path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if we're in a git repository
pub fn is_git_repo() -> Result<bool> {
    is_git_repo_in(None)
}

/// Check if a specific path is in a git repository
pub fn is_git_repo_in(workdir: Option<&Path>) -> Result<bool> {
    let cmd = Cmd::new("git").args(&["rev-parse", "--git-dir"]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    cmd.run_as_check()
}

/// Check if the repository has any commits (HEAD is valid)
#[allow(dead_code)]
pub fn has_commits() -> Result<bool> {
    has_commits_in(None)
}

/// Check if the repository at a specific path has any commits
pub fn has_commits_in(workdir: Option<&Path>) -> Result<bool> {
    let cmd = Cmd::new("git").args(&["rev-parse", "--verify", "--quiet", "HEAD"]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    cmd.run_as_check()
}

/// Get the root directory of the git repository
pub fn get_repo_root() -> Result<PathBuf> {
    get_repo_root_in(None)
}

/// Get the root directory of a git repository in a specific workdir
pub fn get_repo_root_in(workdir: Option<&Path>) -> Result<PathBuf> {
    let cmd = Cmd::new("git").args(&["rev-parse", "--show-toplevel"]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    let path = cmd.run_and_capture_stdout()?;
    Ok(PathBuf::from(path))
}

/// Get the root directory of the git repository containing the given path.
/// Uses `git -C <dir>` to run git from the target directory.
pub fn get_repo_root_for(dir: &Path) -> Result<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output()
        .context("Failed to run git rev-parse")?;

    if !output.status.success() {
        anyhow::bail!("Not a git repository: {}", dir.display());
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(PathBuf::from(path))
}

/// Get the common git directory (shared across all worktrees).
///
/// This returns the absolute path where git stores shared data like refs, objects, and config.
/// - For regular repos: Returns the `.git` directory
/// - For bare repos: Returns the bare repo path (e.g., `.bare`)
///
/// Git commands like `git worktree prune` and `git branch -D` work correctly
/// when run from this directory, even for bare repo setups.
#[allow(dead_code)]
pub fn get_git_common_dir() -> Result<PathBuf> {
    get_git_common_dir_in(None)
}

/// Get the common git directory for a repository at a specific path.
pub fn get_git_common_dir_in(workdir: Option<&Path>) -> Result<PathBuf> {
    let cmd = Cmd::new("git").args(&["rev-parse", "--git-common-dir"]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    let raw = cmd
        .run_and_capture_stdout()
        .context("Failed to get git common directory")?;

    if raw.is_empty() {
        return Err(anyhow!(
            "git rev-parse --git-common-dir returned empty output"
        ));
    }

    let path = PathBuf::from(raw);

    let abs_path = if path.is_relative() {
        let base = match workdir {
            Some(path) => path.to_path_buf(),
            None => std::env::current_dir().context("Failed to get current directory")?,
        };
        base.join(path)
    } else {
        path
    };

    Ok(abs_path)
}
