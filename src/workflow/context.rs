use anyhow::{Context, Result, anyhow};
use std::path::PathBuf;
use std::sync::Arc;

use crate::multiplexer::Multiplexer;
use crate::{config, git};
use tracing::debug;

/// Shared context for workflow operations
///
/// This struct centralizes pre-flight checks and holds essential data
/// needed by workflow modules, reducing code duplication.
pub struct WorkflowContext {
    pub main_worktree_root: PathBuf,
    pub git_common_dir: PathBuf,
    pub main_branch: String,
    pub prefix: String,
    pub config: config::Config,
    pub mux: Arc<dyn Multiplexer>,
    /// Relative path from repo root to config directory.
    /// Empty if config is at repo root or using defaults.
    pub config_rel_dir: PathBuf,
    /// Absolute path to the directory where config was found.
    /// Used as source for file operations (copy/symlink).
    pub config_source_dir: PathBuf,
}

impl WorkflowContext {
    /// Create a new workflow context
    ///
    /// Performs the git repository check and gathers all commonly needed data.
    /// Does NOT check if multiplexer is running or change the current directory - those
    /// are optional operations that can be performed via helper methods.
    pub fn new(
        config: config::Config,
        mux: Arc<dyn Multiplexer>,
        config_location: Option<config::ConfigLocation>,
    ) -> Result<Self> {
        if !git::is_git_repo()? {
            return Err(anyhow!("Not in a git repository"));
        }

        let main_worktree_root =
            git::get_main_worktree_root().context("Could not find the main git worktree")?;

        let git_common_dir =
            git::get_git_common_dir().context("Could not find the git common directory")?;

        let main_branch = if let Some(ref branch) = config.main_branch {
            branch.clone()
        } else {
            git::get_default_branch().context("Failed to determine the main branch")?
        };

        let prefix = config.window_prefix().to_string();

        let (config_rel_dir, config_source_dir) = match config_location {
            Some(loc) => (loc.rel_dir, loc.config_dir),
            None => (PathBuf::new(), main_worktree_root.clone()),
        };

        debug!(
            main_worktree_root = %main_worktree_root.display(),
            git_common_dir = %git_common_dir.display(),
            main_branch = %main_branch,
            prefix = %prefix,
            backend = mux.name(),
            config_rel_dir = %config_rel_dir.display(),
            config_source_dir = %config_source_dir.display(),
            "workflow_context:created"
        );

        Ok(Self {
            main_worktree_root,
            git_common_dir,
            main_branch,
            prefix,
            config,
            mux,
            config_rel_dir,
            config_source_dir,
        })
    }

    /// Ensure the terminal multiplexer is running, returning an error if not
    ///
    /// Call this at the start of workflows that require a multiplexer.
    pub fn ensure_mux_running(&self) -> Result<()> {
        if !self.mux.is_running()? {
            return Err(anyhow!(
                "{} is not running. Please start a {} session first.",
                self.mux.name(),
                self.mux.name()
            ));
        }
        Ok(())
    }

    /// Ensure tmux is running (backward-compat alias for ensure_mux_running)
    #[deprecated(note = "Use ensure_mux_running() instead")]
    #[allow(dead_code)]
    pub fn ensure_tmux_running(&self) -> Result<()> {
        self.ensure_mux_running()
    }

    /// Change working directory to main worktree root
    ///
    /// This is necessary for destructive operations (merge, remove) to prevent
    /// "Unable to read current working directory" errors when the command is run
    /// from within a worktree that is about to be deleted.
    pub fn chdir_to_main_worktree(&self) -> Result<()> {
        debug!(
            safe_cwd = %self.main_worktree_root.display(),
            "workflow_context:changing to main worktree"
        );
        std::env::set_current_dir(&self.main_worktree_root).with_context(|| {
            format!(
                "Could not change directory to '{}'",
                self.main_worktree_root.display()
            )
        })
    }
}
