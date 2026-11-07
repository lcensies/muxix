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

/// Memoized worktree path -> git common dir. A worktree's common dir never
/// changes while the worktree exists, and the daemon resolves it on every
/// sweep for every worktree.
static COMMON_DIRS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, PathBuf>>,
> = std::sync::LazyLock::new(Default::default);

/// Resolve a worktree's git common dir (where `refs/` and `packed-refs` live)
/// without spawning git. Returns `None` if the path is not a worktree.
///
/// - `.git` is a directory: that directory is the common dir.
/// - `.git` is a file (linked worktree): follow `gitdir:`, then its
///   `commondir` pointer.
pub fn common_dir_cached(worktree_path: &Path) -> Option<PathBuf> {
    if let Ok(cache) = COMMON_DIRS.lock()
        && let Some(hit) = cache.get(worktree_path)
    {
        return Some(hit.clone());
    }
    let resolved = resolve_common_dir(worktree_path)?;
    if let Ok(mut cache) = COMMON_DIRS.lock() {
        cache.insert(worktree_path.to_path_buf(), resolved.clone());
    }
    Some(resolved)
}

fn resolve_common_dir(worktree_path: &Path) -> Option<PathBuf> {
    let dot_git = worktree_path.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let gitdir = super::linked_worktree_admin_dir(worktree_path)?;
    match std::fs::read_to_string(gitdir.join("commondir")) {
        Ok(raw) => {
            let rel = Path::new(raw.trim());
            Some(if rel.is_absolute() {
                rel.to_path_buf()
            } else {
                normalize(&gitdir.join(rel))
            })
        }
        // A gitdir without `commondir` is a plain repo gitdir.
        Err(_) => Some(gitdir),
    }
}

/// Collapse `..` components lexically. The paths involved are already
/// absolute, and `canonicalize` would resolve symlinks git itself keeps.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Read a branch's object id straight from ref storage: loose
/// `refs/heads/<branch>` first, then `packed-refs`. Spawns no process.
/// Returns `None` when the branch has no ref.
pub fn read_branch_oid(common_dir: &Path, branch: &str) -> Option<String> {
    if let Ok(raw) = std::fs::read_to_string(common_dir.join("refs/heads").join(branch)) {
        let oid = raw.trim();
        if is_oid(oid) {
            return Some(oid.to_string());
        }
    }

    let want = format!("refs/heads/{branch}");
    let packed = std::fs::read_to_string(common_dir.join("packed-refs")).ok()?;
    for line in packed.lines() {
        // Entries are "<oid> <refname>"; `#` headers and `^<oid>` peel lines
        // carry no branch name.
        if line.starts_with('#') || line.starts_with('^') {
            continue;
        }
        if let Some((oid, name)) = line.split_once(' ')
            && name.trim() == want
            && is_oid(oid)
        {
            return Some(oid.to_string());
        }
    }
    None
}

fn is_oid(s: &str) -> bool {
    !s.is_empty() && s.len() >= 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod ref_tests {
    use super::*;

    fn common_dir(temp: &tempfile::TempDir) -> PathBuf {
        let dir = temp.path().join("repo.git");
        std::fs::create_dir_all(dir.join("refs/heads")).unwrap();
        dir
    }

    #[test]
    fn reads_loose_ref() {
        let temp = tempfile::tempdir().unwrap();
        let dir = common_dir(&temp);
        let oid = "a".repeat(40);
        std::fs::write(dir.join("refs/heads/main"), format!("{oid}\n")).unwrap();
        assert_eq!(read_branch_oid(&dir, "main"), Some(oid));
    }

    #[test]
    fn reads_packed_ref() {
        let temp = tempfile::tempdir().unwrap();
        let dir = common_dir(&temp);
        let oid = "b".repeat(40);
        std::fs::write(
            dir.join("packed-refs"),
            format!(
                "# pack-refs with: peeled fully-peeled sorted \n{oid} refs/heads/main\n^{}\n",
                "c".repeat(40)
            ),
        )
        .unwrap();
        assert_eq!(read_branch_oid(&dir, "main"), Some(oid));
    }

    #[test]
    fn loose_ref_wins_over_packed() {
        let temp = tempfile::tempdir().unwrap();
        let dir = common_dir(&temp);
        let loose = "d".repeat(40);
        std::fs::write(dir.join("refs/heads/main"), &loose).unwrap();
        std::fs::write(
            dir.join("packed-refs"),
            format!("{} refs/heads/main\n", "e".repeat(40)),
        )
        .unwrap();
        assert_eq!(read_branch_oid(&dir, "main"), Some(loose));
    }

    #[test]
    fn missing_ref_is_none() {
        let temp = tempfile::tempdir().unwrap();
        let dir = common_dir(&temp);
        std::fs::write(
            dir.join("packed-refs"),
            format!("{} refs/heads/other\n", "f".repeat(40)),
        )
        .unwrap();
        assert_eq!(read_branch_oid(&dir, "main"), None);
        assert_eq!(read_branch_oid(&dir, "nope"), None);
    }

    #[test]
    fn symbolic_loose_ref_is_not_an_oid() {
        let temp = tempfile::tempdir().unwrap();
        let dir = common_dir(&temp);
        std::fs::write(dir.join("refs/heads/main"), "ref: refs/heads/other\n").unwrap();
        assert_eq!(read_branch_oid(&dir, "main"), None);
    }

    #[test]
    fn common_dir_of_plain_repo_is_dot_git() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("plain");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        assert_eq!(common_dir_cached(&repo), Some(repo.join(".git")));
    }

    #[test]
    fn common_dir_of_linked_worktree_follows_commondir() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main/.git");
        let admin = main.join("worktrees/wt");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::write(admin.join("commondir"), "../..\n").unwrap();
        let wt = temp.path().join("wt");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        assert_eq!(common_dir_cached(&wt), Some(main));
    }

    #[test]
    fn common_dir_of_non_worktree_is_none() {
        let temp = tempfile::tempdir().unwrap();
        assert_eq!(common_dir_cached(temp.path()), None);
    }
}
