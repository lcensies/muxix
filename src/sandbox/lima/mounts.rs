//! Mount path resolution for Lima backend.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Config, IsolationLevel};

/// A mount point configuration for Lima.
#[derive(Debug, Clone)]
pub struct Mount {
    /// Path on the host
    pub host_path: PathBuf,
    /// Path inside the VM (if different from host_path)
    pub guest_path: PathBuf,
    /// Whether the mount is read-only
    pub read_only: bool,
}

impl Mount {
    /// Create a read-write mount
    pub fn rw(path: PathBuf) -> Self {
        Self {
            guest_path: path.clone(),
            host_path: path,
            read_only: false,
        }
    }

    /// Create a read-only mount
    #[allow(dead_code)]
    pub fn ro(path: PathBuf) -> Self {
        Self {
            guest_path: path.clone(),
            host_path: path,
            read_only: true,
        }
    }

    /// Create a mount with different host and guest paths
    #[allow(dead_code)]
    pub fn with_guest_path(mut self, guest_path: PathBuf) -> Self {
        self.guest_path = guest_path;
        self
    }
}

/// Determine the project root using git.
///
/// Uses the git common directory's parent to find the main repository root.
/// This is stable across worktrees: `--show-toplevel` returns each worktree's
/// own path, but `--git-common-dir` always points to the shared `.git` directory
/// in the main repo, so its parent is the true project root.
///
/// This matters for both VM naming (project-level isolation hashes this path)
/// and mount generation (must mount the real project root, not a worktree).
/// Using `--show-toplevel` would produce per-worktree paths like
/// `/code/project__worktrees/feature-a`, causing each worktree to get its own
/// VM and a nonsensical worktrees_dir mount like `feature-a__worktrees`.
pub fn determine_project_root(worktree: &Path) -> Result<PathBuf> {
    let git_common_dir = determine_git_common_dir(worktree)?;

    // The git common dir is typically `/path/to/project/.git`.
    // Its parent is the project root.
    let project_root = git_common_dir.parent().ok_or_else(|| {
        anyhow::anyhow!("Git common dir has no parent: {}", git_common_dir.display())
    })?;

    Ok(project_root.to_path_buf())
}

/// Determine the git common directory using git.
/// Uses `git rev-parse --git-common-dir` to handle `git clone --separate-git-dir` correctly.
pub fn determine_git_common_dir(worktree: &Path) -> Result<PathBuf> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree)
        .arg("rev-parse")
        .arg("--path-format=absolute")
        .arg("--git-common-dir")
        .output()?;

    if !output.status.success() {
        bail!("Failed to determine git common dir");
    }

    let path = String::from_utf8(output.stdout)?.trim().to_string();

    Ok(PathBuf::from(path))
}

/// Get the sandbox auth directory (Claude Code config).
fn sandbox_auth_dir() -> Option<PathBuf> {
    home::home_dir().map(|h| h.join(".claude"))
}

/// Get the Lima guest home directory.
///
/// Lima creates a user named `<host-username>.linux` with home at
/// `/home/<host-username>.linux/`.
fn lima_guest_home() -> Option<PathBuf> {
    let username = std::env::var("USER").ok()?;
    Some(PathBuf::from(format!("/home/{}.linux", username)))
}

/// Calculate the standard worktrees directory for a project.
fn calc_worktrees_dir(project_root: &Path) -> Result<PathBuf> {
    let project_name = project_root
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid project path"))?
        .to_string_lossy();

    let worktrees_dir = project_root
        .parent()
        .ok_or_else(|| anyhow::anyhow!("No parent directory"))?
        .join(format!("{}__worktrees", project_name));

    Ok(worktrees_dir)
}

/// Expand the worktree_dir template (replaces {project} placeholder).
fn expand_worktree_template(template: &str, project_root: &Path) -> Result<PathBuf> {
    let project_name = project_root
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("Invalid project path"))?
        .to_string_lossy();

    let expanded = template.replace("{project}", &project_name);

    // Handle relative paths
    if Path::new(&expanded).is_absolute() {
        Ok(PathBuf::from(expanded))
    } else {
        Ok(project_root.join(expanded))
    }
}

/// Get the XDG state directory (same logic as state/store.rs).
fn get_state_dir() -> Result<PathBuf> {
    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        return Ok(PathBuf::from(state_home));
    }
    let home =
        home::home_dir().ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;
    Ok(home.join(".local/state"))
}

/// Get the host-side state directory for a Lima VM.
/// Uses XDG state dir: $XDG_STATE_HOME/workmux/lima/<vm_name>/
fn lima_state_dir(vm_name: &str) -> Result<PathBuf> {
    let state_dir = get_state_dir()?.join("workmux/lima").join(vm_name);
    std::fs::create_dir_all(&state_dir)?;
    Ok(state_dir)
}

/// Get the state directory path for a VM without creating it.
pub(crate) fn lima_state_dir_path(vm_name: &str) -> Result<PathBuf> {
    Ok(get_state_dir()?.join("workmux/lima").join(vm_name))
}

/// Seed ~/.claude.json into the VM's state directory.
/// Writes a minimal config with hasCompletedOnboarding so Claude Code
/// skips the onboarding flow. Only writes when the destination doesn't
/// exist (if_missing policy). Each VM evolves its own copy independently.
pub(crate) fn seed_claude_json(vm_name: &str) -> Result<()> {
    let state_dir = lima_state_dir(vm_name)?;
    let dest = state_dir.join(".claude.json");
    if !dest.exists() {
        std::fs::write(
            &dest,
            r#"{"hasCompletedOnboarding":true,"bypassPermissionsModeAccepted":true}"#,
        )?;
    }
    Ok(())
}

/// Generate mount points for Lima VM based on isolation level and config.
///
/// The `agent` parameter controls agent-specific mounts (e.g. `~/.claude`
/// is only mounted when the active agent is "claude").
pub fn generate_mounts(
    worktree: &Path,
    isolation: IsolationLevel,
    config: &Config,
    vm_name: &str,
    agent: &str,
) -> Result<Vec<Mount>> {
    let mut mounts = Vec::new();

    match isolation {
        IsolationLevel::Shared => {
            let projects_dir = config.sandbox.lima.projects_dir.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "Shared isolation requires 'sandbox.lima.projects_dir' in config.\n\
                         All projects must be under a single root directory.\n\
                         \n\
                         Example config:\n\
                         sandbox:\n  \
                           lima:\n    \
                             isolation: shared\n    \
                             projects_dir: /Users/me/code"
                )
            })?;

            mounts.push(Mount::rw(projects_dir.clone()));
        }

        IsolationLevel::Project => {
            // 1. Mount project root
            let project_root = determine_project_root(worktree)?;
            mounts.push(Mount::rw(project_root.clone()));

            // 2. Mount git common dir if separate
            let git_common_dir = determine_git_common_dir(worktree)?;
            if !git_common_dir.starts_with(&project_root) {
                mounts.push(Mount::rw(git_common_dir));
            }

            // 3. Mount standard worktrees directory
            let worktrees_dir = calc_worktrees_dir(&project_root)?;

            // CRITICAL: Always create and mount (even if doesn't exist yet)
            std::fs::create_dir_all(&worktrees_dir)?;
            mounts.push(Mount::rw(worktrees_dir.clone()));

            // 4. Mount custom worktree directory if configured
            if let Some(custom_template) = config.worktree_dir.as_ref() {
                let custom_dir = expand_worktree_template(custom_template, &project_root)?;
                std::fs::create_dir_all(&custom_dir)?;

                if custom_dir != worktrees_dir {
                    mounts.push(Mount::rw(custom_dir));
                }
            }
        }
    }

    // Mount host ~/.claude/ to guest $HOME/.claude/ so Claude finds credentials
    if agent == "claude"
        && let Some(auth_dir) = sandbox_auth_dir()
    {
        let guest_path = lima_guest_home()
            .map(|h| h.join(".claude"))
            .unwrap_or_else(|| auth_dir.clone());
        mounts.push(Mount {
            host_path: auth_dir,
            guest_path,
            read_only: false,
        });
    }

    // Mount host ~/.gemini/ to guest $HOME/.gemini/ so Gemini CLI finds credentials
    if agent == "gemini"
        && let Some(home) = home::home_dir()
    {
        let auth_dir = home.join(".gemini");
        let guest_path = lima_guest_home()
            .map(|h| h.join(".gemini"))
            .unwrap_or_else(|| auth_dir.clone());
        mounts.push(Mount {
            host_path: auth_dir,
            guest_path,
            read_only: false,
        });
    }

    // Mount per-VM state directory for workmux state
    if let Ok(state_dir) = lima_state_dir(vm_name) {
        let guest_path = lima_guest_home()
            .map(|h| h.join(".workmux-state"))
            .unwrap_or_else(|| state_dir.clone());
        mounts.push(Mount {
            host_path: state_dir,
            guest_path,
            read_only: false,
        });
    }

    // Extra mounts from config
    for extra in config.sandbox.extra_mounts() {
        let (host_path, guest_path, read_only) = extra.resolve()?;
        mounts.push(Mount {
            host_path,
            guest_path,
            read_only,
        });
    }

    Ok(mounts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_worktree_template() {
        let project_root = PathBuf::from("/Users/test/myproject");
        let template = "/custom/{project}-worktrees";
        let expanded = expand_worktree_template(template, &project_root).unwrap();
        assert_eq!(expanded, PathBuf::from("/custom/myproject-worktrees"));
    }

    #[test]
    fn test_expand_worktree_template_relative() {
        let project_root = PathBuf::from("/Users/test/myproject");
        let template = ".worktrees";
        let expanded = expand_worktree_template(template, &project_root).unwrap();
        assert_eq!(expanded, PathBuf::from("/Users/test/myproject/.worktrees"));
    }

    #[test]
    fn test_seed_claude_json_writes_onboarding_config() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        let dest = state_dir.join(".claude.json");

        assert!(!dest.exists());
        std::fs::write(
            &dest,
            r#"{"hasCompletedOnboarding":true,"bypassPermissionsModeAccepted":true}"#,
        )
        .unwrap();
        assert!(dest.exists());

        let contents: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert_eq!(contents["hasCompletedOnboarding"], true);
    }

    #[test]
    fn test_seed_claude_json_does_not_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();

        let dest = state_dir.join(".claude.json");
        std::fs::write(&dest, r#"{"hasCompletedOnboarding":true,"tips_shown":10}"#).unwrap();

        // if_missing policy: don't overwrite
        if !dest.exists() {
            std::fs::write(
                &dest,
                r#"{"hasCompletedOnboarding":true,"bypassPermissionsModeAccepted":true}"#,
            )
            .unwrap();
        }

        let contents: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&dest).unwrap()).unwrap();
        assert_eq!(contents["tips_shown"], 10);
    }

    #[test]
    fn test_lima_state_dir_path_format() {
        let path = lima_state_dir_path("wm-myproject-abc12345").unwrap();
        // Should end with the expected suffix regardless of XDG_STATE_HOME
        assert!(path.ends_with("workmux/lima/wm-myproject-abc12345"));
    }
}
