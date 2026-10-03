use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

use crate::cmd::Cmd;
use crate::config::MuxMode;

use super::WorktreeNotFound;
use super::branch::unset_branch_upstream_in;

/// Check if a worktree already exists for a branch
#[allow(dead_code)]
pub fn worktree_exists(branch_name: &str) -> Result<bool> {
    worktree_exists_in(branch_name, None)
}

/// Check if a worktree already exists for a branch in a specific workdir
pub fn worktree_exists_in(branch_name: &str, workdir: Option<&Path>) -> Result<bool> {
    match get_worktree_path_in(branch_name, workdir) {
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
#[allow(dead_code)]
pub fn create_worktree(
    worktree_path: &Path,
    branch_name: &str,
    create_branch: bool,
    base_branch: Option<&str>,
    track_upstream: bool,
) -> Result<()> {
    create_worktree_in(
        worktree_path,
        branch_name,
        create_branch,
        base_branch,
        track_upstream,
        None,
    )
}

/// Create a new git worktree from a specific workdir
pub fn create_worktree_in(
    worktree_path: &Path,
    branch_name: &str,
    create_branch: bool,
    base_branch: Option<&str>,
    track_upstream: bool,
    workdir: Option<&Path>,
) -> Result<()> {
    let path_str = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("Invalid worktree path"))?;

    let mut cmd = Cmd::new("git").arg("worktree").arg("add");
    if let Some(path) = workdir {
        cmd = cmd.workdir(path);
    }

    if create_branch {
        cmd = cmd.arg("-b").arg(branch_name).arg(path_str);
        if let Some(base) = base_branch {
            cmd = cmd.arg(base);
        }
    } else {
        cmd = cmd.arg(path_str).arg(branch_name);
    }

    cmd.run().context("Failed to create worktree")?;

    if create_branch && !track_upstream {
        unset_branch_upstream_in(branch_name, workdir)?;
    }

    Ok(())
}

/// Move a registered worktree to a new path using `git worktree move`.
///
/// Git updates the worktree admin dir's `gitdir` file and the worktree's
/// `.git` pointer. Note: the admin dir itself (`.git/worktrees/<basename>/`)
/// keeps its original basename; muxix does not rely on that path shape.
pub fn move_worktree_in(old_path: &Path, new_path: &Path, workdir: Option<&Path>) -> Result<()> {
    let old = old_path
        .to_str()
        .ok_or_else(|| anyhow!("Invalid old worktree path"))?;
    let new = new_path
        .to_str()
        .ok_or_else(|| anyhow!("Invalid new worktree path"))?;
    let cmd = Cmd::new("git").args(&["worktree", "move", old, new]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    cmd.run()
        .with_context(|| format!("Failed to move worktree {} -> {}", old, new))?;
    Ok(())
}

/// Migrate all `muxix.worktree.<old_handle>.*` config entries to
/// `muxix.worktree.<new_handle>.*`, then remove the old section.
pub fn migrate_worktree_meta_in(
    old_handle: &str,
    new_handle: &str,
    workdir: Option<&Path>,
) -> Result<()> {
    if old_handle == new_handle {
        return Ok(());
    }
    let git = || {
        let cmd = Cmd::new("git");
        match workdir {
            Some(path) => cmd.workdir(path),
            None => cmd,
        }
    };
    let old_section = format!("muxix.worktree.{}", old_handle);
    let regex_pattern = format!(r"^{}\.", regex::escape(&old_section));
    let output = git()
        .args(&["config", "--local", "--get-regexp", &regex_pattern])
        .run_and_capture_stdout()
        .unwrap_or_default();

    for line in output.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let Some(suffix) = key.strip_prefix(&format!("{}.", old_section)) else {
            continue;
        };
        let new_key = format!("muxix.worktree.{}.{}", new_handle, suffix);
        git()
            .args(&["config", "--local", &new_key, value])
            .run()
            .with_context(|| format!("Failed to set {}", new_key))?;
    }

    // Remove the old section (ignore "no such section" errors).
    let _ = git()
        .args(&["config", "--local", "--remove-section", &old_section])
        .run();

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

/// Whether a path recorded on a task is still this repository's worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingState {
    /// git lists it as a worktree of this repository.
    Live,
    /// git does not list it, and nothing is at that path.
    Missing,
    /// Something exists at that path but git does not call it ours. Never removed:
    /// a recorded path that drifted onto someone else's directory must not be
    /// deleted on the strength of a stale record.
    Foreign,
}

/// Resolve a path as far as the filesystem allows. Recorded paths may not exist,
/// so a failed canonicalize is not an error — it just leaves the path as written.
fn resolved(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Classify a recorded worktree path against git's own registry.
///
/// git is the authority here rather than a naming convention: once the agent picks
/// worktree names, a path derived from a task id proves nothing about what is
/// actually on disk or which repository owns it.
pub fn binding_state(recorded: &Path, workdir: Option<&Path>) -> Result<BindingState> {
    let want = resolved(recorded);
    if list_worktrees_in(workdir)?
        .iter()
        .any(|(path, _)| resolved(path) == want)
    {
        return Ok(BindingState::Live);
    }
    Ok(if recorded.exists() {
        BindingState::Foreign
    } else {
        BindingState::Missing
    })
}

/// Get the path to a worktree for a given branch
#[allow(dead_code)]
pub fn get_worktree_path(branch_name: &str) -> Result<PathBuf> {
    get_worktree_path_in(branch_name, None)
}

/// Get the path to a worktree for a given branch in a specific workdir
pub fn get_worktree_path_in(branch_name: &str, workdir: Option<&Path>) -> Result<PathBuf> {
    let worktrees = list_worktrees_in(workdir)?;

    for (path, branch) in worktrees {
        if branch == branch_name {
            if !path.exists() {
                return Err(WorktreeNotFound(branch_name.to_string()).into());
            }
            return Ok(path);
        }
    }

    Err(WorktreeNotFound(branch_name.to_string()).into())
}

/// Find a worktree by handle (directory name) or branch name.
/// Tries handle first, then falls back to branch lookup.
/// Returns both the path and the branch name checked out in that worktree.
pub fn find_worktree(name: &str) -> Result<(PathBuf, String)> {
    find_worktree_in(name, None)
}

/// Find a worktree by handle or branch name in a specific workdir.
pub fn find_worktree_in(name: &str, workdir: Option<&Path>) -> Result<(PathBuf, String)> {
    let worktrees = list_worktrees_in(workdir)?;

    // First: try to match by handle (directory name)
    for (path, branch) in &worktrees {
        if let Some(dir_name) = path.file_name()
            && dir_name.to_string_lossy() == name
        {
            if !path.exists() {
                return Err(WorktreeNotFound(name.to_string()).into());
            }
            return Ok((path.clone(), branch.clone()));
        }
    }

    // Fallback: try to match by branch name
    for (path, branch) in worktrees {
        if branch == name {
            if !path.exists() {
                return Err(WorktreeNotFound(name.to_string()).into());
            }
            return Ok((path, branch));
        }
    }

    Err(WorktreeNotFound(name.to_string()).into())
}

/// List all worktrees with their branches
pub fn list_worktrees() -> Result<Vec<(PathBuf, String)>> {
    list_worktrees_in(None)
}

/// List all worktrees with their branches, optionally in a specific workdir
pub fn list_worktrees_in(workdir: Option<&Path>) -> Result<Vec<(PathBuf, String)>> {
    let cmd = Cmd::new("git").args(&["worktree", "list", "--porcelain"]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    let list = cmd
        .run_and_capture_stdout()
        .context("Failed to list worktrees")?;
    parse_worktree_list_porcelain(&list)
}

/// Store per-worktree metadata in git config.
#[allow(dead_code)]
pub fn set_worktree_meta(handle: &str, key: &str, value: &str) -> Result<()> {
    set_worktree_meta_in(handle, key, value, None)
}

/// Store per-worktree metadata in git config in a specific workdir.
pub fn set_worktree_meta_in(
    handle: &str,
    key: &str,
    value: &str,
    workdir: Option<&Path>,
) -> Result<()> {
    let config_key = format!("muxix.worktree.{}.{}", handle, key);
    let cmd = Cmd::new("git").args(&["config", "--local", &config_key, value]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    cmd.run()
        .with_context(|| format!("Failed to set worktree metadata {}.{}", handle, key))?;
    Ok(())
}

/// Retrieve per-worktree metadata from git config.
/// Returns None if the key doesn't exist.
#[allow(dead_code)]
pub fn get_worktree_meta(handle: &str, key: &str) -> Option<String> {
    get_worktree_meta_in(handle, key, None)
}

/// Retrieve per-worktree metadata from git config in a specific workdir.
pub fn get_worktree_meta_in(handle: &str, key: &str, workdir: Option<&Path>) -> Option<String> {
    let config_key = format!("muxix.worktree.{}.{}", handle, key);
    let cmd = Cmd::new("git").args(&["config", "--local", "--get", &config_key]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    cmd.run_and_capture_stdout().ok().filter(|s| !s.is_empty())
}

pub fn get_worktree_target_window(handle: &str) -> Option<String> {
    get_worktree_target_window_in(handle, None)
}

pub fn get_worktree_target_window_in(handle: &str, workdir: Option<&Path>) -> Option<String> {
    get_worktree_meta_in(handle, "target-window", workdir)
}

pub fn get_worktree_target_session(handle: &str) -> Option<String> {
    get_worktree_target_session_in(handle, None)
}

pub fn get_worktree_target_session_in(handle: &str, workdir: Option<&Path>) -> Option<String> {
    get_worktree_meta_in(handle, "target-session", workdir)
}

pub fn get_worktree_window_session(handle: &str) -> Option<String> {
    get_worktree_window_session_in(handle, None)
}

pub fn get_worktree_window_session_in(handle: &str, workdir: Option<&Path>) -> Option<String> {
    get_worktree_meta_in(handle, "window-session", workdir)
}

/// Determine the tmux mode for a worktree from git metadata.
/// Returns None if no metadata is found (legacy worktree).
pub fn get_worktree_mode_opt(handle: &str) -> Option<MuxMode> {
    get_worktree_mode_opt_in(handle, None)
}

/// Determine the tmux mode for a worktree from git metadata in a specific workdir.
pub fn get_worktree_mode_opt_in(handle: &str, workdir: Option<&Path>) -> Option<MuxMode> {
    match get_worktree_meta_in(handle, "mode", workdir) {
        Some(mode) if mode == "session" => Some(MuxMode::Session),
        Some(mode) if mode == "window" => Some(MuxMode::Window),
        _ => None,
    }
}

/// Determine the tmux mode for a worktree from git metadata.
/// Falls back to Window mode if no metadata is found (backward compatibility).
pub fn get_worktree_mode(handle: &str) -> MuxMode {
    get_worktree_mode_in(handle, None)
}

/// [`get_worktree_mode`], reading metadata from an explicit directory instead
/// of the process CWD.
pub fn get_worktree_mode_in(handle: &str, workdir: Option<&Path>) -> MuxMode {
    get_worktree_mode_opt_in(handle, workdir).unwrap_or(MuxMode::Window)
}

pub fn get_all_worktree_meta_key_in(
    workdir: Option<&Path>,
    key_name: &str,
) -> std::collections::HashMap<String, String> {
    let pattern = format!(r"^muxix\.worktree\..*\.{}$", regex::escape(key_name));
    let cmd = Cmd::new("git").args(&["config", "--local", "--get-regexp", &pattern]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    let output = cmd.run_and_capture_stdout().unwrap_or_default();

    let mut values = std::collections::HashMap::new();
    let suffix = format!(".{}", key_name);
    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let key = parts[0];
            let value = parts[1].trim();
            if let Some(rest) = key.strip_prefix("muxix.worktree.")
                && let Some(handle) = rest.strip_suffix(&suffix)
            {
                values.insert(handle.to_string(), value.to_string());
            }
        }
    }
    values
}

/// Batch-load all worktree modes, optionally in a specific workdir.
pub fn get_all_worktree_modes_in(
    workdir: Option<&Path>,
) -> std::collections::HashMap<String, MuxMode> {
    let cmd = Cmd::new("git").args(&[
        "config",
        "--local",
        "--get-regexp",
        r"^muxix\.worktree\..*\.mode$",
    ]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    let output = cmd.run_and_capture_stdout().unwrap_or_default();

    let mut modes = std::collections::HashMap::new();
    for line in output.lines() {
        // Format: "muxix.worktree.<handle>.mode <value>"
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() == 2 {
            let key = parts[0];
            let value = parts[1].trim();
            // Extract handle from "muxix.worktree.<handle>.mode"
            if let Some(rest) = key.strip_prefix("muxix.worktree.")
                && let Some(handle) = rest.strip_suffix(".mode")
            {
                let mode = if value == "session" {
                    MuxMode::Session
                } else {
                    MuxMode::Window
                };
                modes.insert(handle.to_string(), mode);
            }
        }
    }
    modes
}

/// Remove all metadata for a worktree handle.
pub fn remove_worktree_meta_in(handle: &str, workdir: Option<&Path>) -> Result<()> {
    // Use --remove-section to remove all keys under the handle's section
    let section = format!("muxix.worktree.{}", handle);
    let cmd = Cmd::new("git").args(&["config", "--local", "--remove-section", &section]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    let _ = cmd.run();
    Ok(())
}

/// Get the main worktree root directory (not a linked worktree)
///
/// For bare repositories with linked worktrees, this returns the bare repo path.
/// For regular repositories, this returns the first worktree that exists on disk.
pub fn get_main_worktree_root() -> Result<PathBuf> {
    get_main_worktree_root_in(None)
}

/// Get the main worktree root directory from a specific workdir
pub fn get_main_worktree_root_in(workdir: Option<&Path>) -> Result<PathBuf> {
    let cmd = Cmd::new("git").args(&["worktree", "list", "--porcelain"]);
    let cmd = match workdir {
        Some(path) => cmd.workdir(path),
        None => cmd,
    };
    let list_str = cmd
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use std::sync::Mutex;

    static CWD_LOCK: Mutex<()> = Mutex::new(());

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .expect("git command should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        let output = Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir)
            .output()
            .expect("git init should run");
        assert!(
            output.status.success(),
            "git init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        run_git(dir, &["config", "user.email", "test@example.com"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "test\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
    }

    #[test]
    fn porcelain_parser_reads_branches_and_detached_heads() {
        let listed = parse_worktree_list_porcelain(
            "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\
             \n\
             worktree /repo/../wm-a\nHEAD def\nbranch refs/heads/feat/login\n\
             \n\
             worktree /repo/../wm-b\nHEAD 123\ndetached\n",
        )
        .unwrap();

        assert_eq!(
            listed,
            vec![
                (PathBuf::from("/repo"), "main".to_string()),
                (PathBuf::from("/repo/../wm-a"), "feat/login".to_string()),
                (PathBuf::from("/repo/../wm-b"), "(detached)".to_string()),
            ]
        );
    }

    #[test]
    fn binding_state_classifies_live_missing_and_foreign_paths() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        init_repo(&repo);

        let tree = dir.path().join("wm-live");
        run_git(
            &repo,
            &["worktree", "add", "-b", "feat/live", tree.to_str().unwrap()],
        );

        assert_eq!(
            binding_state(&tree, Some(&repo)).unwrap(),
            BindingState::Live
        );
        assert_eq!(
            binding_state(&dir.path().join("never-existed"), Some(&repo)).unwrap(),
            BindingState::Missing
        );

        // A directory that exists but is not one of this repo's worktrees. Cleanup
        // driven by a stale record would otherwise delete it.
        let stranger = dir.path().join("not-ours");
        std::fs::create_dir(&stranger).unwrap();
        assert_eq!(
            binding_state(&stranger, Some(&repo)).unwrap(),
            BindingState::Foreign
        );
    }

    #[test]
    fn create_worktree_in_uses_explicit_repo_not_process_cwd() {
        let _guard = CWD_LOCK.lock().unwrap();
        let original_cwd = std::env::current_dir().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let repo_a = temp.path().join("repo-a");
        let repo_b = temp.path().join("repo-b");
        std::fs::create_dir_all(&repo_a).unwrap();
        std::fs::create_dir_all(&repo_b).unwrap();
        init_repo(&repo_a);
        init_repo(&repo_b);

        std::env::set_current_dir(&repo_a).unwrap();
        let result = (|| {
            let worktree_path = temp.path().join("repo-b__worktrees").join("feature");
            create_worktree_in(
                &worktree_path,
                "feature",
                true,
                Some("main"),
                false,
                Some(&repo_b),
            )
            .unwrap();
            set_worktree_meta_in("feature", "mode", "window", Some(&repo_b)).unwrap();

            let repo_b_worktrees = Command::new("git")
                .current_dir(&repo_b)
                .args(["worktree", "list", "--porcelain"])
                .output()
                .unwrap();
            assert!(repo_b_worktrees.status.success());
            let repo_b_list = String::from_utf8(repo_b_worktrees.stdout).unwrap();
            assert!(repo_b_list.contains("branch refs/heads/feature"));
            assert_eq!(
                get_worktree_meta_in("feature", "mode", Some(&repo_b)).as_deref(),
                Some("window")
            );

            let repo_a_has_branch = Command::new("git")
                .current_dir(&repo_a)
                .args(["rev-parse", "--verify", "--quiet", "feature"])
                .status()
                .unwrap()
                .success();
            assert!(!repo_a_has_branch);
            assert_eq!(
                std::env::current_dir().unwrap(),
                repo_a.canonicalize().unwrap()
            );
        })();
        std::env::set_current_dir(original_cwd).unwrap();
        result
    }
}
