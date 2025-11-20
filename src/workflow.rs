use anyhow::{Context, Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};
use std::{thread, time::Duration};

use crate::{cli, cmd, config, git, tmux};
use tracing::{debug, info, trace, warn};

use fs_extra::dir as fs_dir;
use fs_extra::file as fs_file;

const WINDOW_CLOSE_DELAY_MS: u64 = 300;

/// Result of creating a worktree
pub struct CreateResult {
    pub worktree_path: PathBuf,
    pub branch_name: String,
    pub post_create_hooks_run: usize,
    pub base_branch: Option<String>,
}

/// Result of merging a worktree
pub struct MergeResult {
    pub branch_merged: String,
    pub main_branch: String,
    pub had_staged_changes: bool,
}

/// Result of removing a worktree
pub struct RemoveResult {
    pub branch_removed: String,
}

/// Result of cleanup operations
pub struct CleanupResult {
    pub tmux_window_killed: bool,
    pub worktree_removed: bool,
    pub local_branch_deleted: bool,
    pub remote_branch_deleted: bool,
    pub remote_delete_error: Option<String>,
    pub ran_inside_target_window: bool,
}

/// Options for setting up a worktree environment
#[derive(Debug, Clone)]
pub struct SetupOptions {
    pub run_hooks: bool,
    pub run_file_ops: bool,
    pub run_pane_commands: bool,
    pub prompt_file_path: Option<PathBuf>,
    /// If true, switch to the new tmux window when done; if false, leave it in the background.
    pub focus_window: bool,
}

impl SetupOptions {
    /// Create SetupOptions with all options enabled
    #[allow(dead_code)]
    pub fn all() -> Self {
        Self {
            run_hooks: true,
            run_file_ops: true,
            run_pane_commands: true,
            prompt_file_path: None,
            focus_window: true,
        }
    }

    /// Create SetupOptions with custom values
    pub fn new(run_hooks: bool, run_file_ops: bool, run_pane_commands: bool) -> Self {
        Self {
            run_hooks,
            run_file_ops,
            run_pane_commands,
            prompt_file_path: None,
            focus_window: true,
        }
    }

    /// Create SetupOptions with a prompt file
    #[allow(dead_code)]
    pub fn with_prompt(
        run_hooks: bool,
        run_file_ops: bool,
        run_pane_commands: bool,
        prompt_file_path: Option<PathBuf>,
    ) -> Self {
        Self {
            run_hooks,
            run_file_ops,
            run_pane_commands,
            prompt_file_path,
            focus_window: true,
        }
    }
}

/// Create a new worktree with tmux window and panes
pub fn create(
    branch_name: &str,
    base_branch: Option<&str>,
    remote_branch: Option<&str>,
    prompt: Option<&cli::Prompt>,
    config: &config::Config,
    options: SetupOptions,
    agent: Option<&str>,
) -> Result<CreateResult> {
    info!(
        branch = branch_name,
        base = ?base_branch,
        remote = ?remote_branch,
        "create:start"
    );

    // Validate pane config before any other operations
    if let Some(panes) = &config.panes {
        config::validate_panes_config(panes)?;
    }

    // Pre-flight checks
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    if !tmux::is_running()? {
        return Err(anyhow!(
            "tmux is not running. Please start a tmux session first."
        ));
    }

    let prefix = config.window_prefix();
    if tmux::window_exists(prefix, branch_name)? {
        return Err(anyhow!(
            "A tmux window named '{}' already exists",
            branch_name
        ));
    }

    if git::worktree_exists(branch_name)? {
        return Err(anyhow!(
            "A worktree for branch '{}' already exists. Use 'workmux open {}' to open it.",
            branch_name,
            branch_name
        ));
    }

    // Auto-detect: create branch if it doesn't exist
    let branch_exists = git::branch_exists(branch_name)?;
    if branch_exists && remote_branch.is_some() {
        return Err(anyhow!(
            "Branch '{}' already exists. Remove '--remote' or pick a different branch name.",
            branch_name
        ));
    }
    let create_new = !branch_exists;
    let mut track_upstream = false;
    debug!(
        branch = branch_name,
        branch_exists, create_new, "create:branch detection"
    );

    // Determine the base for the new branch
    let base_branch_for_creation = if let Some(remote_spec) = remote_branch {
        let spec = git::parse_remote_branch_spec(remote_spec)?;
        if !git::remote_exists(&spec.remote)? {
            return Err(anyhow!(
                "Remote '{}' does not exist. Available remotes: {:?}",
                spec.remote,
                git::list_remotes()?
            ));
        }
        git::fetch_remote(&spec.remote)
            .with_context(|| format!("Failed to fetch from remote '{}'", spec.remote))?;
        let remote_ref = format!("{}/{}", spec.remote, spec.branch);
        if !git::branch_exists(&remote_ref)? {
            return Err(anyhow!(
                "Remote branch '{}' was not found. Double-check the name or fetch it manually.",
                remote_ref
            ));
        }
        track_upstream = true;
        Some(remote_ref)
    } else if create_new {
        if let Some(base) = base_branch {
            // Use the explicitly provided base branch/commit/tag
            Some(base.to_string())
        } else {
            // Default to the current branch when no explicit base was provided
            let current_branch = git::get_current_branch()
                .context("Failed to determine the current branch to use as the base")?;
            let current_branch = current_branch.trim().to_string();

            if current_branch.is_empty() {
                return Err(anyhow!(
                    "Cannot determine current branch (detached HEAD). \
                     Use --base to explicitly specify the starting point."
                ));
            }

            Some(current_branch)
        }
    } else {
        None
    };

    // Determine worktree path: use config.worktree_dir or default to <project>__worktrees pattern
    let repo_root = git::get_repo_root()?;
    let base_dir = if let Some(ref worktree_dir) = config.worktree_dir {
        let path = Path::new(worktree_dir);
        if path.is_absolute() {
            // Use absolute path as-is
            path.to_path_buf()
        } else {
            // Relative path: resolve from repo root
            repo_root.join(path)
        }
    } else {
        // Default behavior: <project_root>/../<project_name>__worktrees
        let project_name = repo_root
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| anyhow!("Could not determine project name"))?;
        repo_root
            .parent()
            .ok_or_else(|| anyhow!("Could not determine parent directory"))?
            .join(format!("{}__worktrees", project_name))
    };
    let worktree_path = base_dir.join(branch_name);

    // Create worktree
    info!(
        branch = branch_name,
        path = %worktree_path.display(),
        create_new,
        base = ?base_branch_for_creation,
        "create:creating worktree"
    );

    git::create_worktree(
        &worktree_path,
        branch_name,
        create_new,
        base_branch_for_creation.as_deref(),
        track_upstream,
    )
    .context("Failed to create git worktree")?;

    // Store the base branch in git config for future reference (used during removal checks)
    if let Some(ref base) = base_branch_for_creation {
        git::set_branch_base(branch_name, base).with_context(|| {
            format!(
                "Failed to store base branch '{}' for branch '{}'",
                base, branch_name
            )
        })?;
        debug!(
            branch = branch_name,
            base = base,
            "create:stored base branch in git config"
        );
    }

    // Setup the rest of the environment (tmux, files, hooks)
    let prompt_file_path = if let Some(p) = prompt {
        Some(write_prompt_file(branch_name, p)?)
    } else {
        None
    };

    // Merge prompt file path into options
    let options_with_prompt = SetupOptions {
        prompt_file_path,
        ..options
    };
    let mut result = setup_environment(
        branch_name,
        &worktree_path,
        config,
        &options_with_prompt,
        agent,
    )?;
    result.base_branch = base_branch_for_creation.clone();
    info!(
        branch = branch_name,
        path = %result.worktree_path.display(),
        hooks_run = result.post_create_hooks_run,
        "create:completed"
    );
    Ok(result)
}

/// Open a tmux window for an existing worktree
pub fn open(
    branch_name: &str,
    config: &config::Config,
    options: SetupOptions,
) -> Result<CreateResult> {
    info!(
        branch = branch_name,
        run_hooks = options.run_hooks,
        run_file_ops = options.run_file_ops,
        "open:start"
    );

    // Validate pane config before any other operations
    if let Some(panes) = &config.panes {
        config::validate_panes_config(panes)?;
    }

    // Pre-flight checks
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    if !tmux::is_running()? {
        return Err(anyhow!(
            "tmux is not running. Please start a tmux session first."
        ));
    }

    let prefix = config.window_prefix();
    if tmux::window_exists(prefix, branch_name)? {
        return Err(anyhow!(
            "A tmux window named '{}' already exists. To switch to it, run: tmux select-window -t '{}'",
            branch_name,
            tmux::prefixed(prefix, branch_name)
        ));
    }

    // This command requires the worktree to already exist
    let worktree_path = git::get_worktree_path(branch_name).with_context(|| {
        format!(
            "No worktree found for branch '{}'. Use 'workmux add {}' to create it.",
            branch_name, branch_name
        )
    })?;

    // Setup the environment
    let result = setup_environment(branch_name, &worktree_path, config, &options, None)?;
    info!(
        branch = branch_name,
        path = %result.worktree_path.display(),
        hooks_run = result.post_create_hooks_run,
        "open:completed"
    );
    Ok(result)
}

/// Sets up the tmux window, files, and hooks for a worktree.
/// This is the shared logic between `create` and `open`.
fn setup_environment(
    branch_name: &str,
    worktree_path: &Path,
    config: &config::Config,
    options: &SetupOptions,
    agent: Option<&str>,
) -> Result<CreateResult> {
    debug!(
        branch = branch_name,
        path = %worktree_path.display(),
        run_hooks = options.run_hooks,
        run_file_ops = options.run_file_ops,
        "setup_environment:start"
    );
    let prefix = config.window_prefix();
    let repo_root = git::get_repo_root()?;

    // Perform file operations (copy and symlink) if requested
    if options.run_file_ops {
        handle_file_operations(&repo_root, worktree_path, &config.files)
            .context("Failed to perform file operations")?;
        debug!(
            branch = branch_name,
            "setup_environment:file operations applied"
        );
    }

    // Run post-create hooks before opening tmux so the new window appears "ready"
    let mut hooks_run = 0;
    if options.run_hooks
        && let Some(post_create) = &config.post_create
        && !post_create.is_empty()
    {
        hooks_run = post_create.len();
        for (idx, command) in post_create.iter().enumerate() {
            info!(branch = branch_name, step = idx + 1, total = hooks_run, command = %command, "setup_environment:hook start");
            println!("  [{}/{}] Running: {}", idx + 1, hooks_run, command);
            cmd::shell_command(command, worktree_path)
                .with_context(|| format!("Failed to run post-create command: '{}'", command))?;
            info!(branch = branch_name, step = idx + 1, total = hooks_run, command = %command, "setup_environment:hook complete");
        }
        info!(
            branch = branch_name,
            total = hooks_run,
            "setup_environment:hooks complete"
        );
    }

    // Create tmux window once prep work is finished
    tmux::create_window(
        prefix,
        branch_name,
        worktree_path,
        /* detached: */ !options.focus_window,
    )
    .context("Failed to create tmux window")?;
    info!(
        branch = branch_name,
        "setup_environment:tmux window created"
    );

    // Setup panes
    let panes = config.panes.as_deref().unwrap_or(&[]);
    let resolved_panes = resolve_pane_configuration(panes, agent);
    let pane_setup_result = tmux::setup_panes(
        prefix,
        branch_name,
        &resolved_panes,
        worktree_path,
        tmux::PaneSetupOptions {
            run_commands: options.run_pane_commands,
            prompt_file_path: options.prompt_file_path.as_deref(),
        },
        config,
        agent,
    )
    .context("Failed to setup panes")?;
    debug!(
        branch = branch_name,
        focus_index = pane_setup_result.focus_pane_index,
        "setup_environment:panes configured"
    );

    // Focus the configured pane and optionally switch to the window
    if options.focus_window {
        tmux::select_pane(prefix, branch_name, pane_setup_result.focus_pane_index)?;
        tmux::select_window(prefix, branch_name)?;
    } else {
        // Background mode: do not steal focus from the current window.
        // We intentionally skip select_window to keep the user's current window.
    }

    Ok(CreateResult {
        worktree_path: worktree_path.to_path_buf(),
        branch_name: branch_name.to_string(),
        post_create_hooks_run: hooks_run,
        base_branch: None,
    })
}

fn resolve_pane_configuration(
    original_panes: &[config::PaneConfig],
    agent: Option<&str>,
) -> Vec<config::PaneConfig> {
    let Some(agent_cmd) = agent else {
        return original_panes.to_vec();
    };

    if original_panes
        .iter()
        .any(|pane| pane.command.as_deref() == Some("<agent>"))
    {
        return original_panes.to_vec();
    }

    let mut panes = original_panes.to_vec();

    if let Some(focused) = panes.iter_mut().find(|pane| pane.focus) {
        focused.command = Some(agent_cmd.to_string());
        return panes;
    }

    if let Some(first) = panes.get_mut(0) {
        first.command = Some(agent_cmd.to_string());
        return panes;
    }

    vec![config::PaneConfig {
        command: Some(agent_cmd.to_string()),
        focus: true,
        split: None,
        size: None,
        percentage: None,
        target: None,
    }]
}

/// Performs copy and symlink operations from the repo root to the worktree
fn handle_file_operations(
    repo_root: &Path,
    worktree_path: &Path,
    file_config: &config::FileConfig,
) -> Result<()> {
    debug!(
        repo = %repo_root.display(),
        worktree = %worktree_path.display(),
        copy_patterns = file_config.copy.as_ref().map(|v| v.len()).unwrap_or(0),
        symlink_patterns = file_config.symlink.as_ref().map(|v| v.len()).unwrap_or(0),
        "file_operations:start"
    );

    let canon_repo_root = repo_root.canonicalize().with_context(|| {
        format!(
            "Failed to canonicalize repository root path: {:?}",
            repo_root
        )
    })?;

    // Handle copies
    if let Some(copy_patterns) = &file_config.copy {
        for pattern in copy_patterns {
            let full_pattern = repo_root.join(pattern).to_string_lossy().to_string();
            for entry in glob::glob(&full_pattern)? {
                let source_path = entry?;

                // Validate that the resolved source path stays within the repository root
                let canon_source_path = source_path.canonicalize().with_context(|| {
                    format!("Failed to canonicalize source path: {:?}", source_path)
                })?;
                if !canon_source_path.starts_with(&canon_repo_root) {
                    return Err(anyhow!(
                        "Path traversal detected for copy pattern '{}'. The resolved path '{}' is outside the repository root.",
                        pattern,
                        source_path.display()
                    ));
                }

                let relative_path = source_path.strip_prefix(repo_root).with_context(|| {
                    format!(
                        "Path '{}' is outside the repository root '{}', which is not allowed.",
                        source_path.display(),
                        repo_root.display()
                    )
                })?;
                let dest_path = worktree_path.join(relative_path);

                if source_path.is_dir() {
                    // Create destination parent directory
                    if let Some(parent) = dest_path.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    // Use fs_extra::dir::copy which handles recursion and symlinks correctly
                    let mut dir_options = fs_dir::CopyOptions::new();
                    dir_options.overwrite = true;
                    dir_options.content_only = true;
                    fs::create_dir_all(&dest_path)?; // Ensure dest exists
                    fs_dir::copy(&source_path, &dest_path, &dir_options).with_context(|| {
                        format!(
                            "Failed to copy directory {:?} to {:?}",
                            source_path, dest_path
                        )
                    })?;
                    trace!(
                        from = %source_path.display(),
                        to = %dest_path.display(),
                        "file_operations:copied directory"
                    );
                } else {
                    // Copy single file
                    if let Some(parent) = dest_path.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!("Failed to create parent directory for {:?}", dest_path)
                        })?;
                    }
                    let mut options = fs_file::CopyOptions::new();
                    options.overwrite = true;
                    fs_file::copy(&source_path, &dest_path, &options).with_context(|| {
                        format!("Failed to copy file {:?} to {:?}", source_path, dest_path)
                    })?;
                    trace!(
                        from = %source_path.display(),
                        to = %dest_path.display(),
                        "file_operations:copied file"
                    );
                }
            }
        }
    }

    // Handle symlinks
    if let Some(symlink_patterns) = &file_config.symlink {
        for pattern in symlink_patterns {
            let full_pattern = repo_root.join(pattern).to_string_lossy().to_string();
            for entry in glob::glob(&full_pattern)? {
                let source_path = entry?;

                // Validate that the resolved source path is within the repository root
                let canon_source_path = source_path.canonicalize().with_context(|| {
                    format!("Failed to canonicalize source path: {:?}", source_path)
                })?;
                if !canon_source_path.starts_with(&canon_repo_root) {
                    return Err(anyhow!(
                        "Path traversal detected for symlink pattern '{}'. The resolved path '{}' is outside the repository root.",
                        pattern,
                        source_path.display()
                    ));
                }

                let relative_path = source_path.strip_prefix(repo_root)?;
                let dest_path = worktree_path.join(relative_path);

                if let Some(parent) = dest_path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("Failed to create parent directory for {:?}", dest_path)
                    })?;
                }

                // Critical: create a relative path for the symlink
                let dest_parent = dest_path.parent().ok_or_else(|| {
                    anyhow!(
                        "Could not determine parent directory for destination path: {:?}",
                        dest_path
                    )
                })?;

                let relative_source = pathdiff::diff_paths(&source_path, dest_parent)
                    .ok_or_else(|| anyhow!("Could not create relative path for symlink"))?;

                // Remove existing file/symlink at destination to avoid errors
                // IMPORTANT: Use symlink_metadata to avoid following symlinks
                if let Ok(metadata) = dest_path.symlink_metadata() {
                    if metadata.is_dir() {
                        fs::remove_dir_all(&dest_path).with_context(|| {
                            format!("Failed to remove existing directory at {:?}", &dest_path)
                        })?;
                    } else {
                        // Handles both files and symlinks
                        fs::remove_file(&dest_path).with_context(|| {
                            format!("Failed to remove existing file/symlink at {:?}", &dest_path)
                        })?;
                    }
                }

                #[cfg(unix)]
                std::os::unix::fs::symlink(&relative_source, &dest_path).with_context(|| {
                    format!(
                        "Failed to create symlink from {:?} to {:?}",
                        relative_source, dest_path
                    )
                })?;

                #[cfg(windows)]
                {
                    if source_path.is_dir() {
                        std::os::windows::fs::symlink_dir(&relative_source, &dest_path)
                    } else {
                        std::os::windows::fs::symlink_file(&relative_source, &dest_path)
                    }
                    .with_context(|| {
                        format!(
                            "Failed to create symlink from {:?} to {:?}",
                            relative_source, dest_path
                        )
                    })?;
                }
                trace!(
                    from = %relative_source.display(),
                    to = %dest_path.display(),
                    "file_operations:symlinked"
                );
            }
        }
    }

    Ok(())
}

/// Merge a branch into the main branch and clean up
pub fn merge(
    branch_name: Option<&str>,
    ignore_uncommitted: bool,
    delete_remote: bool,
    rebase: bool,
    squash: bool,
    config: &config::Config,
) -> Result<MergeResult> {
    info!(
        branch = ?branch_name,
        ignore_uncommitted,
        delete_remote,
        rebase,
        squash,
        "merge:start"
    );
    // Pre-flight checks
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    // Change CWD to main worktree to prevent errors if the command is run from within
    // the worktree that is about to be deleted.
    let main_worktree_root = git::get_main_worktree_root()
        .context("Could not find main worktree to run merge operations")?;
    debug!(safe_cwd = %main_worktree_root.display(), "merge:changing to main worktree");
    std::env::set_current_dir(&main_worktree_root).with_context(|| {
        format!(
            "Could not change directory to '{}'",
            main_worktree_root.display()
        )
    })?;

    // Determine the branch to merge
    let branch_to_merge = if let Some(name) = branch_name {
        name.to_string()
    } else {
        // Running from within a worktree - get current branch
        git::get_current_branch().context("Failed to get current branch")?
    };

    // Get worktree path for the branch to be merged
    let worktree_path = git::get_worktree_path(&branch_to_merge)
        .with_context(|| format!("No worktree found for branch '{}'", branch_to_merge))?;
    debug!(
        branch = branch_to_merge,
        path = %worktree_path.display(),
        "merge:worktree resolved"
    );

    // Handle changes in the source worktree
    if git::has_unstaged_changes(&worktree_path)? && !ignore_uncommitted {
        return Err(anyhow!(
            "Worktree for '{}' has unstaged changes. Please stage or stash them, or use --ignore-uncommitted.",
            branch_to_merge
        ));
    }

    let had_staged_changes = git::has_staged_changes(&worktree_path)?;
    if had_staged_changes && !ignore_uncommitted {
        // Commit using git's editor (respects $EDITOR or git config)
        info!(path = %worktree_path.display(), "merge:committing staged changes");
        git::commit_with_editor(&worktree_path).context("Failed to commit staged changes")?;
    }

    // Get the main branch (from config or auto-detect)
    let main_branch = config
        .main_branch
        .as_ref()
        .map(|s| Ok(s.clone()))
        .unwrap_or_else(git::get_default_branch)
        .context("Failed to determine the main branch. Specify it in .workmux.yaml")?;

    if branch_to_merge == main_branch {
        return Err(anyhow!("Cannot merge the main branch into itself."));
    }
    debug!(
        branch = branch_to_merge,
        main = &main_branch,
        "merge:main branch resolved"
    );

    // Get the main worktree path. This is the canonical, non-linked worktree.
    let main_worktree_path =
        git::get_main_worktree_root().context("Failed to find the main worktree")?;

    // Safety check: Abort if the main worktree has uncommitted changes
    if git::has_uncommitted_changes(&main_worktree_path)? {
        return Err(anyhow!(
            "Main worktree has uncommitted changes. Please commit or stash them before merging."
        ));
    }

    // Explicitly switch to the main branch to ensure correct merge target
    git::switch_branch_in_worktree(&main_worktree_path, &main_branch)?;

    if rebase {
        // Rebase the feature branch on top of main inside its own worktree.
        // This is where conflicts will be detected.
        println!("Rebasing '{}' onto '{}'...", &branch_to_merge, &main_branch);
        info!(
            branch = branch_to_merge,
            base = &main_branch,
            "merge:rebase start"
        );
        git::rebase_branch_onto_base(&worktree_path, &main_branch).with_context(|| {
            format!(
                "Rebase failed, likely due to conflicts.\n\n\
                Please resolve them manually inside the worktree at '{}'.\n\
                Then, run 'git rebase --continue' to proceed or 'git rebase --abort' to cancel.",
                worktree_path.display()
            )
        })?;

        // After a successful rebase, merge into main. This will be a fast-forward.
        git::merge_in_worktree(&main_worktree_path, &branch_to_merge)
            .context("Failed to merge rebased branch. This should have been a fast-forward.")?;
        info!(branch = branch_to_merge, "merge:fast-forward complete");
    } else if squash {
        // Perform the squash merge. This stages all changes from the feature branch but does not commit.
        git::merge_squash_in_worktree(&main_worktree_path, &branch_to_merge)
            .context("Failed to perform squash merge")?;

        // Prompt the user to provide a commit message for the squashed changes.
        println!("Staged squashed changes. Please provide a commit message in your editor.");
        git::commit_with_editor(&main_worktree_path)
            .context("Failed to commit squashed changes. You may need to commit them manually.")?;
        info!(branch = branch_to_merge, "merge:squash merge committed");
    } else {
        // Default merge commit workflow
        git::merge_in_worktree(&main_worktree_path, &branch_to_merge)
            .context("Failed to merge branch")?;
        info!(branch = branch_to_merge, "merge:standard merge complete");
    }

    // Always force cleanup after a successful merge
    let prefix = config.window_prefix();
    info!(
        branch = branch_to_merge,
        delete_remote, "merge:cleanup start"
    );
    let cleanup_result = cleanup(
        prefix,
        &branch_to_merge,
        &worktree_path,
        true,
        delete_remote,
        false, // keep_branch: always delete when merging
        config,
    )?;

    // Navigate to the main branch window if it exists
    if tmux::is_running()? && tmux::window_exists(prefix, &main_branch)? {
        tmux::select_window(prefix, &main_branch)?;
    }

    schedule_window_close_if_needed(prefix, &branch_to_merge, &cleanup_result);

    Ok(MergeResult {
        branch_merged: branch_to_merge,
        main_branch,
        had_staged_changes,
    })
}

/// Remove a worktree without merging
pub fn remove(
    branch_name: &str,
    force: bool,
    delete_remote: bool,
    keep_branch: bool,
    config: &config::Config,
) -> Result<RemoveResult> {
    info!(
        branch = branch_name,
        force, delete_remote, keep_branch, "remove:start"
    );
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    // Change CWD to main worktree to prevent errors if the command is run from within
    // the worktree that is about to be deleted.
    let main_worktree_root = git::get_main_worktree_root()
        .context("Could not find main worktree to run remove operations")?;
    debug!(safe_cwd = %main_worktree_root.display(), "remove:changing to main worktree");
    std::env::set_current_dir(&main_worktree_root).with_context(|| {
        format!(
            "Could not change directory to '{}'",
            main_worktree_root.display()
        )
    })?;

    // Get worktree path - this also validates that the worktree exists
    let worktree_path = git::get_worktree_path(branch_name)
        .with_context(|| format!("No worktree found for branch '{}'", branch_name))?;
    debug!(branch = branch_name, path = %worktree_path.display(), "remove:worktree resolved");

    // Safety Check: Prevent deleting the main worktree itself, regardless of branch.
    let is_main_worktree = match (
        worktree_path.canonicalize(),
        main_worktree_root.canonicalize(),
    ) {
        (Ok(canon_wt_path), Ok(canon_main_path)) => {
            // Best case: both paths exist and can be resolved. This is the most reliable check.
            canon_wt_path == canon_main_path
        }
        _ => {
            // Fallback: If canonicalization fails on either path (e.g., directory was
            // manually removed, broken symlink), compare the raw paths provided by git.
            // This is a critical safety net.
            worktree_path == main_worktree_root
        }
    };

    if is_main_worktree {
        return Err(anyhow!(
            "Cannot remove branch '{}' because it is checked out in the main worktree at '{}'. \
            Switch the main worktree to a different branch first, or create a linked worktree for '{}'.",
            branch_name,
            main_worktree_root.display(),
            branch_name
        ));
    }

    // Safety Check: Prevent deleting the main branch by name (secondary check)
    let main_branch = git::get_default_branch()
        .context("Failed to determine the main branch. You can specify it in .workmux.yaml")?;
    if branch_name == main_branch {
        return Err(anyhow!("Cannot delete the main branch ('{}')", main_branch));
    }

    if worktree_path.exists() && git::has_uncommitted_changes(&worktree_path)? && !force {
        return Err(anyhow!(
            "Worktree has uncommitted changes. Use --force to delete anyway."
        ));
    }

    // Note: Unmerged branch check removed - git branch -d/D handles this natively
    // The CLI provides a user-friendly confirmation prompt before calling this function
    let prefix = config.window_prefix();
    info!(
        branch = branch_name,
        delete_remote, keep_branch, "remove:cleanup start"
    );
    let cleanup_result = cleanup(
        prefix,
        branch_name,
        &worktree_path,
        force,
        delete_remote,
        keep_branch,
        config,
    )?;

    // Navigate to the main branch window if it exists
    if tmux::is_running()? && tmux::window_exists(prefix, &main_branch)? {
        tmux::select_window(prefix, &main_branch)?;
    }

    schedule_window_close_if_needed(prefix, branch_name, &cleanup_result);

    Ok(RemoveResult {
        branch_removed: branch_name.to_string(),
    })
}

/// Centralized function to clean up tmux and git resources
pub fn cleanup(
    prefix: &str,
    branch_name: &str,
    worktree_path: &Path,
    force: bool,
    delete_remote: bool,
    keep_branch: bool,
    config: &config::Config,
) -> Result<CleanupResult> {
    info!(
        branch = branch_name,
        path = %worktree_path.display(),
        force,
        delete_remote,
        keep_branch,
        "cleanup:start"
    );
    // Change the CWD to main worktree before any destructive operations.
    // This prevents "Unable to read current working directory" errors when the command
    // is run from within the worktree being deleted.
    let main_worktree_root = git::get_main_worktree_root()
        .context("Could not find main worktree to run cleanup operations")?;
    debug!(safe_cwd = %main_worktree_root.display(), "cleanup:changing to main worktree");
    std::env::set_current_dir(&main_worktree_root).with_context(|| {
        format!(
            "Could not change directory to '{}'",
            main_worktree_root.display()
        )
    })?;

    let tmux_running = tmux::is_running().unwrap_or(false);
    let running_inside_target_window = if tmux_running {
        match tmux::current_window_name() {
            Ok(Some(current_name)) => current_name == tmux::prefixed(prefix, branch_name),
            _ => false,
        }
    } else {
        false
    };

    let mut result = CleanupResult {
        tmux_window_killed: false,
        worktree_removed: false,
        local_branch_deleted: false,
        remote_branch_deleted: false,
        remote_delete_error: None,
        ran_inside_target_window: running_inside_target_window,
    };

    // Helper closure to perform the actual filesystem and git cleanup.
    // This avoids code duplication while enforcing the correct operational order.
    let perform_fs_git_cleanup = |result: &mut CleanupResult| -> Result<()> {
        // Run pre-delete hooks before removing the worktree directory
        if let Some(pre_delete_hooks) = &config.pre_delete {
            info!(
                branch = branch_name,
                count = pre_delete_hooks.len(),
                "cleanup:running pre-delete hooks"
            );
            for command in pre_delete_hooks {
                // Run the hook with the worktree path as the working directory.
                // This allows for relative paths like `node_modules` in the command.
                cmd::shell_command(command, worktree_path)
                    .with_context(|| format!("Failed to run pre-delete command: '{}'", command))?;
            }
        }

        // 1. Forcefully remove the worktree directory from the filesystem.
        if worktree_path.exists() {
            std::fs::remove_dir_all(worktree_path).with_context(|| {
                format!(
                    "Failed to remove worktree directory at {}. \
                Please close any terminals or editors using this directory and try again.",
                    worktree_path.display()
                )
            })?;
            result.worktree_removed = true;
            info!(branch = branch_name, path = %worktree_path.display(), "cleanup:worktree directory removed");
        }

        // Clean up the prompt file if it exists
        let prompt_file = PathBuf::from(format!("/tmp/workmux-prompt-{}.md", branch_name));
        if prompt_file.exists() {
            if let Err(e) = std::fs::remove_file(&prompt_file) {
                warn!(path = %prompt_file.display(), error = %e, "cleanup:failed to remove prompt file");
            } else {
                debug!(path = %prompt_file.display(), "cleanup:prompt file removed");
            }
        }

        // 2. Prune worktrees to clean up git's metadata.
        git::prune_worktrees().context("Failed to prune worktrees")?;
        debug!("cleanup:git worktrees pruned");

        // 3. Delete the local branch (unless keeping it).
        if !keep_branch {
            git::delete_branch(branch_name, force).context("Failed to delete local branch")?;
            result.local_branch_deleted = true;
            info!(branch = branch_name, "cleanup:local branch deleted");
        }

        // 4. Delete the remote branch if requested (redundant check due to CLI conflict, but safe).
        if delete_remote && !keep_branch {
            match git::delete_remote_branch(branch_name) {
                Ok(_) => {
                    result.remote_branch_deleted = true;
                    info!(branch = branch_name, "cleanup:remote branch deleted");
                }
                Err(e) => {
                    warn!(branch = branch_name, error = %e, "cleanup:failed to delete remote branch");
                    result.remote_delete_error = Some(e.to_string());
                }
            }
        }
        Ok(())
    };

    if running_inside_target_window {
        info!(
            branch = branch_name,
            "cleanup:deferring tmux window kill because command is running inside the window"
        );
        // Perform all filesystem and git cleanup *before* returning. The caller
        // will then schedule the asynchronous window close.
        perform_fs_git_cleanup(&mut result)?;
    } else {
        // Not running inside the target window, so we kill the window first
        // to release any shell locks on the directory.
        if tmux_running && tmux::window_exists(prefix, branch_name).unwrap_or(false) {
            tmux::kill_window(prefix, branch_name).context("Failed to kill tmux window")?;
            result.tmux_window_killed = true;
            info!(branch = branch_name, "cleanup:tmux window killed");

            // Poll to confirm the window is gone before proceeding. This prevents a race
            // condition where we try to delete the directory before the shell inside
            // the tmux window has terminated.
            const MAX_RETRIES: u32 = 20;
            const RETRY_DELAY: Duration = Duration::from_millis(50);
            let mut window_is_gone = false;
            for _ in 0..MAX_RETRIES {
                if !tmux::window_exists(prefix, branch_name)? {
                    window_is_gone = true;
                    break;
                }
                thread::sleep(RETRY_DELAY);
            }

            if !window_is_gone {
                warn!(
                    branch = branch_name,
                    "cleanup:tmux window did not close within retry budget"
                );
                eprintln!(
                    "Warning: tmux window for '{}' did not close in the allotted time. \
                    Filesystem cleanup may fail.",
                    branch_name
                );
            }
        }
        // Now that the window is gone, it's safe to clean up the filesystem and git state.
        perform_fs_git_cleanup(&mut result)?;
    }

    Ok(result)
}

fn schedule_window_close_if_needed(
    prefix: &str,
    branch_name: &str,
    cleanup_result: &CleanupResult,
) {
    if !cleanup_result.ran_inside_target_window {
        return;
    }

    let delay = Duration::from_millis(WINDOW_CLOSE_DELAY_MS);
    match tmux::schedule_window_close(prefix, branch_name, delay) {
        Ok(_) => info!(branch = branch_name, "cleanup:tmux window close scheduled"),
        Err(e) => warn!(
            branch = branch_name,
            error = %e,
            "cleanup:failed to schedule tmux window close",
        ),
    }
}

/// List all worktrees with their status
pub struct WorktreeInfo {
    pub branch: String,
    pub path: PathBuf,
    pub has_tmux: bool,
    pub has_unmerged: bool,
}

pub fn list(config: &config::Config) -> Result<Vec<WorktreeInfo>> {
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    let worktrees_data = git::list_worktrees()?;

    if worktrees_data.is_empty() {
        return Ok(Vec::new());
    }

    // Check tmux status and get all windows once to avoid repeated process calls
    let tmux_windows: std::collections::HashSet<String> = if tmux::is_running().unwrap_or(false) {
        tmux::get_all_window_names().unwrap_or_default()
    } else {
        std::collections::HashSet::new()
    };

    // Get the main branch for unmerged checks
    let main_branch = git::get_default_branch().ok();

    // Get all unmerged branches in one go for efficiency
    // Prefer checking against remote tracking branch for more accurate results
    let unmerged_branches = main_branch
        .as_deref()
        .and_then(|main| git::get_merge_base(main).ok())
        .and_then(|base| git::get_unmerged_branches(&base).ok())
        .unwrap_or_default(); // Use an empty set on failure

    let prefix = config.window_prefix();
    let worktrees: Vec<WorktreeInfo> = worktrees_data
        .into_iter()
        .map(|(path, branch)| {
            let prefixed_branch_name = tmux::prefixed(prefix, &branch);
            let has_tmux = tmux_windows.contains(&prefixed_branch_name);

            // Check for unmerged commits, but only if this isn't the main branch
            let has_unmerged = if let Some(ref main) = main_branch {
                if branch == *main || branch == "(detached)" {
                    false
                } else {
                    unmerged_branches.contains(&branch)
                }
            } else {
                false
            };

            WorktreeInfo {
                branch,
                path,
                has_tmux,
                has_unmerged,
            }
        })
        .collect();

    Ok(worktrees)
}
fn write_prompt_file(branch_name: &str, prompt: &cli::Prompt) -> Result<PathBuf> {
    let content = match prompt {
        cli::Prompt::Inline(text) => text.clone(),
        cli::Prompt::FromFile(path) => fs::read_to_string(path)
            .with_context(|| format!("Failed to read prompt file '{}'", path.display()))?,
    };

    // Write to /tmp instead of the worktree to avoid polluting git status
    let prompt_path = PathBuf::from(format!("/tmp/workmux-prompt-{}.md", branch_name));
    fs::write(&prompt_path, content)
        .with_context(|| format!("Failed to write prompt file '{}'", prompt_path.display()))?;
    Ok(prompt_path)
}

/// Create a new worktree and move uncommitted changes from the current worktree into it.
pub fn create_with_changes(
    branch_name: &str,
    include_untracked: bool,
    patch: bool,
    config: &config::Config,
    options: SetupOptions,
) -> Result<CreateResult> {
    info!(
        branch = branch_name,
        include_untracked, patch, "create_with_changes:start"
    );

    // Pre-flight Checks
    if !git::is_git_repo()? {
        return Err(anyhow!("Not in a git repository"));
    }

    let original_worktree_path = git::get_repo_root()?;

    // Check for changes based on the include_untracked flag
    let has_tracked_changes = git::has_tracked_changes(&original_worktree_path)?;
    let has_movable_untracked =
        include_untracked && git::has_untracked_files(&original_worktree_path)?;

    if !has_tracked_changes && !has_movable_untracked {
        return Err(anyhow!(
            "No uncommitted changes to move. Use 'workmux add {}' to create a clean worktree.",
            branch_name
        ));
    }

    if git::branch_exists(branch_name)? {
        return Err(anyhow!("Branch '{}' already exists.", branch_name));
    }

    // 1. Stash changes
    let stash_message = format!("workmux: moving changes to {}", branch_name);
    git::stash_push(&stash_message, include_untracked, patch)
        .context("Failed to stash current changes")?;
    info!(branch = branch_name, "create_with_changes: changes stashed");

    // 2. Create new worktree
    let create_result = match create(branch_name, None, None, None, config, options, None) {
        Ok(result) => result,
        Err(e) => {
            warn!(error = %e, "create_with_changes: worktree creation failed, popping stash");
            // Best effort to restore the stash - if this fails, user still has stash@{0}
            let _ = git::stash_pop(&original_worktree_path);
            return Err(e).context(
                "Failed to create new worktree. Stashed changes have been restored if possible.",
            );
        }
    };

    let new_worktree_path = &create_result.worktree_path;
    info!(
        path = %new_worktree_path.display(),
        "create_with_changes: worktree created"
    );

    // 3. Apply stash in new worktree
    match git::stash_pop(new_worktree_path) {
        Ok(_) => {
            // 4. Success: Clean up original worktree
            info!("create_with_changes: stash applied successfully, cleaning original worktree");
            git::reset_hard(&original_worktree_path)?;

            info!(
                branch = branch_name,
                "create_with_changes: completed successfully"
            );
            Ok(create_result)
        }
        Err(e) => {
            // 5. Failure: Rollback
            warn!(error = %e, "create_with_changes: failed to apply stash, rolling back");

            remove(branch_name, true, false, false, config).context(
                "Rollback failed: could not clean up the new worktree. Please do so manually.",
            )?;

            Err(anyhow!(
                "Could not apply changes to '{}', likely due to conflicts.\n\n\
                The new worktree has been removed.\n\
                Your changes are safe in the latest stash. Run 'git stash pop' manually to resolve.",
                branch_name
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_pane_configuration_no_agent_returns_original() {
        let original_panes = vec![config::PaneConfig {
            command: Some("vim".to_string()),
            focus: true,
            split: None,
            size: None,
            percentage: None,
            target: None,
        }];

        let result = resolve_pane_configuration(&original_panes, None);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].command, Some("vim".to_string()));
    }

    #[test]
    fn resolve_pane_configuration_agent_placeholder_returns_original() {
        let original_panes = vec![config::PaneConfig {
            command: Some("<agent>".to_string()),
            focus: true,
            split: None,
            size: None,
            percentage: None,
            target: None,
        }];

        let result = resolve_pane_configuration(&original_panes, Some("claude"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].command, Some("<agent>".to_string()));
    }

    #[test]
    fn resolve_pane_configuration_agent_sets_focused_pane() {
        let original_panes = vec![
            config::PaneConfig {
                command: Some("vim".to_string()),
                focus: false,
                split: None,
                size: None,
                percentage: None,
                target: None,
            },
            config::PaneConfig {
                command: Some("npm run dev".to_string()),
                focus: true,
                split: None,
                size: None,
                percentage: None,
                target: None,
            },
        ];

        let result = resolve_pane_configuration(&original_panes, Some("claude"));
        assert_eq!(result[0].command, Some("vim".to_string()));
        assert_eq!(result[1].command, Some("claude".to_string()));
    }

    #[test]
    fn resolve_pane_configuration_agent_sets_first_pane_when_no_focus() {
        let original_panes = vec![config::PaneConfig {
            command: Some("vim".to_string()),
            focus: false,
            split: None,
            size: None,
            percentage: None,
            target: None,
        }];

        let result = resolve_pane_configuration(&original_panes, Some("claude"));
        assert_eq!(result[0].command, Some("claude".to_string()));
    }

    #[test]
    fn resolve_pane_configuration_agent_creates_new_pane_when_empty() {
        let result = resolve_pane_configuration(&[], Some("claude"));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].command, Some("claude".to_string()));
        assert!(result[0].focus);
    }
}
