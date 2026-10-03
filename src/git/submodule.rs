use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

use crate::cmd::Cmd;

/// Information about a git submodule
#[derive(Debug, Clone)]
pub struct Submodule {
    /// The submodule name (from [submodule "<name>"] in .gitmodules)
    pub name: String,
    /// The relative path within the repo where the submodule is checked out
    pub path: String,
}

/// List all submodules defined in .gitmodules for a repo.
/// Returns empty vec if .gitmodules does not exist or has no submodules.
pub fn list_submodules(repo_root: &Path) -> Result<Vec<Submodule>> {
    let output = Cmd::new("git")
        .workdir(repo_root)
        .args(&[
            "config",
            "--file",
            ".gitmodules",
            "--get-regexp",
            r"submodule\..*\.path",
        ])
        .run_and_capture_stdout()
        .unwrap_or_default();

    if output.is_empty() {
        return Ok(Vec::new());
    }

    let mut submodules = Vec::new();
    for line in output.lines() {
        let parts: Vec<&str> = line.splitn(2, ' ').collect();
        if parts.len() != 2 {
            continue;
        }
        let key = parts[0]; // "submodule.<name>.path"
        let path = parts[1].trim().to_string();

        let name = key
            .strip_prefix("submodule.")
            .and_then(|s| s.strip_suffix(".path"))
            .unwrap_or(&path)
            .to_string();

        submodules.push(Submodule { name, path });
    }

    Ok(submodules)
}

/// Check whether a branch exists in a submodule repo (identified by its working dir).
pub fn submodule_branch_exists(submodule_in_main: &Path, branch_name: &str) -> bool {
    Cmd::new("git")
        .workdir(submodule_in_main)
        .args(&["rev-parse", "--verify", "--quiet", branch_name])
        .run_as_check()
        .unwrap_or(false)
}

/// Create a git worktree for a submodule inside a parent worktree.
///
/// The worktree lands at `parent_worktree_path/<submodule.path>`, replacing the
/// empty/uninitialised submodule directory that a fresh parent worktree contains.
///
/// * `main_repo_root`       – root of the main git repo (where .gitmodules lives)
/// * `parent_worktree_path` – path of the already-created parent worktree
/// * `submodule`            – submodule descriptor
/// * `branch_name`          – branch to check out in the submodule worktree
/// * `base_branch`          – branch to fork from when `branch_name` does not exist yet
pub fn create_submodule_worktree(
    main_repo_root: &Path,
    parent_worktree_path: &Path,
    submodule: &Submodule,
    branch_name: &str,
    base_branch: Option<&str>,
) -> Result<()> {
    let submodule_in_main = main_repo_root.join(&submodule.path);

    // Ensure the submodule is initialised in the main repo so that its embedded
    // git dir (at .git/modules/<name>) is present.
    if !submodule_in_main.join(".git").exists() {
        Cmd::new("git")
            .workdir(main_repo_root)
            .args(&["submodule", "update", "--init", &submodule.path])
            .run()
            .with_context(|| format!("Failed to init submodule '{}'", submodule.name))?;
    }

    let submodule_wt_path = parent_worktree_path.join(&submodule.path);

    // The parent worktree may contain an empty directory at the submodule path
    // (git worktree add leaves submodule dirs uninitialised). Remove it so that
    // `git worktree add` can use that path.
    if submodule_wt_path.exists() {
        // Only remove if it is empty or is just an empty dir (not a real checkout).
        let git_file = submodule_wt_path.join(".git");
        if !git_file.exists() {
            std::fs::remove_dir_all(&submodule_wt_path).with_context(|| {
                format!(
                    "Failed to remove submodule placeholder '{}'",
                    submodule_wt_path.display()
                )
            })?;
        }
    }

    // Ensure parent directories exist (in case the submodule path has subdirs).
    if let Some(parent) = submodule_wt_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create parent dir for submodule worktree '{}'",
                parent.display()
            )
        })?;
    }

    let wt_path_str = submodule_wt_path
        .to_str()
        .ok_or_else(|| anyhow!("Invalid submodule worktree path"))?;

    if submodule_branch_exists(&submodule_in_main, branch_name) {
        Cmd::new("git")
            .workdir(&submodule_in_main)
            .args(&["worktree", "add", wt_path_str, branch_name])
            .run()
            .with_context(|| {
                format!(
                    "Failed to create submodule worktree for '{}' on branch '{}'",
                    submodule.name, branch_name
                )
            })?;
    } else {
        let base = base_branch.unwrap_or("HEAD");
        Cmd::new("git")
            .workdir(&submodule_in_main)
            .args(&["worktree", "add", "-b", branch_name, wt_path_str, base])
            .run()
            .with_context(|| {
                format!(
                    "Failed to create submodule worktree for '{}' with new branch '{}'",
                    submodule.name, branch_name
                )
            })?;
    }

    Ok(())
}

/// Remove a submodule worktree.
///
/// Runs `git worktree remove` from the submodule's main-repo checkout so that
/// git can resolve the worktree's admin dir correctly.
pub fn remove_submodule_worktree(
    main_repo_root: &Path,
    submodule: &Submodule,
    submodule_wt_path: &Path,
    force: bool,
) -> Result<()> {
    let submodule_in_main = main_repo_root.join(&submodule.path);

    if !submodule_in_main.exists() {
        // Submodule not initialised in main repo; fall back to plain directory removal.
        if submodule_wt_path.exists() {
            std::fs::remove_dir_all(submodule_wt_path).with_context(|| {
                format!(
                    "Failed to remove submodule worktree dir '{}'",
                    submodule_wt_path.display()
                )
            })?;
        }
        return Ok(());
    }

    let path_str = submodule_wt_path
        .to_str()
        .ok_or_else(|| anyhow!("Invalid submodule worktree path"))?;

    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(path_str);

    Cmd::new("git")
        .workdir(&submodule_in_main)
        .args(&args)
        .run()
        .with_context(|| {
            format!(
                "Failed to remove submodule worktree '{}' for submodule '{}'",
                submodule_wt_path.display(),
                submodule.name
            )
        })?;

    Ok(())
}

/// Detect which submodules have an active worktree inside a parent worktree.
///
/// A submodule path that contains a `.git` *file* (not a directory) indicates a
/// linked worktree rather than a normal `git submodule update` checkout.
pub fn get_submodule_worktrees(
    main_repo_root: &Path,
    parent_worktree_path: &Path,
) -> Result<Vec<(Submodule, PathBuf)>> {
    let submodules = list_submodules(main_repo_root)?;
    let mut result = Vec::new();

    for submodule in submodules {
        let wt_path = parent_worktree_path.join(&submodule.path);
        let git_marker = wt_path.join(".git");
        // A .git file (not directory) means this is a linked worktree
        if git_marker.is_file() {
            result.push((submodule, wt_path));
        }
    }

    Ok(result)
}

/// Merge a submodule feature branch into a target branch.
///
/// Performs the merge inside the submodule's main-repo checkout (not inside the
/// worktree), then returns so the caller can stage the updated submodule pointer
/// in the parent repo.
pub fn merge_submodule_worktree(
    main_repo_root: &Path,
    submodule: &Submodule,
    branch_name: &str,
    target_branch: &str,
) -> Result<()> {
    let submodule_in_main = main_repo_root.join(&submodule.path);

    Cmd::new("git")
        .workdir(&submodule_in_main)
        .args(&["checkout", target_branch])
        .run()
        .with_context(|| {
            format!(
                "Failed to switch submodule '{}' to branch '{}'",
                submodule.name, target_branch
            )
        })?;

    Cmd::new("git")
        .workdir(&submodule_in_main)
        .args(&["merge", branch_name])
        .run()
        .with_context(|| {
            format!(
                "Failed to merge '{}' into '{}' in submodule '{}'",
                branch_name, target_branch, submodule.name
            )
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn run_git(repo: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(repo)
            .args(args)
            .output()
            .expect("git command should run");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(dir)
            .output()
            .expect("git init");
        run_git(dir, &["config", "user.email", "ZPvOdfJ@PwBVrFx.info"]);
        run_git(dir, &["config", "user.name", "Test User"]);
        std::fs::write(dir.join("README.md"), "submodule test\n").unwrap();
        run_git(dir, &["add", "README.md"]);
        run_git(dir, &["commit", "-m", "initial"]);
    }

    fn setup_parent_with_submodule(
        temp: &tempfile::TempDir,
    ) -> (std::path::PathBuf, std::path::PathBuf) {
        let sub_repo = temp.path().join("sub-repo");
        let parent_repo = temp.path().join("parent-repo");
        std::fs::create_dir_all(&sub_repo).unwrap();
        std::fs::create_dir_all(&parent_repo).unwrap();
        init_repo(&sub_repo);
        init_repo(&parent_repo);

        // Add sub-repo as submodule "proto" in parent.
        // Pass protocol.file.allow=always via -c to allow local file:// transport.
        let out = Command::new("git")
            .current_dir(&parent_repo)
            .args([
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                sub_repo.to_str().unwrap(),
                "proto",
            ])
            .output()
            .expect("git submodule add");
        assert!(
            out.status.success(),
            "git submodule add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        run_git(&parent_repo, &["commit", "-m", "add submodule"]);

        (parent_repo, sub_repo)
    }

    #[test]
    fn list_submodules_returns_entries() {
        let temp = tempfile::tempdir().unwrap();
        let (parent_repo, _sub_repo) = setup_parent_with_submodule(&temp);

        let submodules = list_submodules(&parent_repo).unwrap();
        assert_eq!(submodules.len(), 1);
        assert_eq!(submodules[0].name, "proto");
        assert_eq!(submodules[0].path, "proto");
    }

    #[test]
    fn list_submodules_empty_for_plain_repo() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);

        let submodules = list_submodules(&repo).unwrap();
        assert!(submodules.is_empty());
    }

    #[test]
    fn create_submodule_worktree_creates_linked_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let (parent_repo, _sub_repo) = setup_parent_with_submodule(&temp);

        // Create a parent worktree
        let parent_wt = temp.path().join("parent-repo__worktrees").join("feature");
        std::fs::create_dir_all(&parent_wt).unwrap();
        run_git(
            &parent_repo,
            &[
                "worktree",
                "add",
                parent_wt.to_str().unwrap(),
                "-b",
                "feature",
                "main",
            ],
        );

        let submodules = list_submodules(&parent_repo).unwrap();
        let submodule = &submodules[0];

        // Create submodule worktree
        create_submodule_worktree(&parent_repo, &parent_wt, submodule, "feature", Some("main"))
            .unwrap();

        let submodule_wt_path = parent_wt.join("proto");
        // Should be a linked worktree: .git is a file
        assert!(submodule_wt_path.join(".git").is_file());
    }

    #[test]
    fn remove_submodule_worktree_cleans_up() {
        let temp = tempfile::tempdir().unwrap();
        let (parent_repo, _sub_repo) = setup_parent_with_submodule(&temp);

        let parent_wt = temp.path().join("parent-repo__worktrees").join("feature");
        run_git(
            &parent_repo,
            &[
                "worktree",
                "add",
                parent_wt.to_str().unwrap(),
                "-b",
                "feature",
                "main",
            ],
        );

        let submodules = list_submodules(&parent_repo).unwrap();
        let submodule = &submodules[0];

        create_submodule_worktree(&parent_repo, &parent_wt, submodule, "feature", Some("main"))
            .unwrap();

        let submodule_wt_path = parent_wt.join("proto");
        assert!(submodule_wt_path.exists());

        remove_submodule_worktree(&parent_repo, submodule, &submodule_wt_path, true).unwrap();
        assert!(!submodule_wt_path.exists());
    }
}
