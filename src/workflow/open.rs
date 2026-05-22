use anyhow::{Context, Result, anyhow};
use regex::Regex;

use crate::git;
use crate::multiplexer::util::prefixed;
use crate::multiplexer::{MuxHandle, WindowTarget};
use crate::prompt::Prompt;
use tracing::info;

use super::context::WorkflowContext;
use super::setup;
use super::types::{CreateResult, SetupOptions};
use crate::config::MuxMode;

/// Open a tmux window for an existing worktree
pub fn open(
    name: &str,
    context: &WorkflowContext,
    options: SetupOptions,
    new_window: bool,
    mode_override: Option<MuxMode>,
    prompt_file_only: Option<&Prompt>,
) -> Result<CreateResult> {
    info!(
        name = name,
        run_hooks = options.run_hooks,
        run_file_ops = options.run_file_ops,
        new_window = new_window,
        mode_override = ?mode_override,
        "open:start"
    );

    // Validate mutual exclusion of panes/windows config (mode-independent)
    if context.config.panes.is_some() && context.config.windows.is_some() {
        anyhow::bail!("Cannot specify both 'panes' and 'windows' in configuration.");
    }
    if let Some(panes) = &context.config.panes {
        crate::config::validate_panes_config(panes)?;
    }

    // Pre-flight checks
    context.ensure_mux_running()?;

    // This command requires the worktree to already exist
    // Smart resolution: try handle first, then branch name
    let (worktree_path, branch_name) = git::find_worktree(name).map_err(|_| {
        anyhow!(
            "Worktree '{}' not found. Use 'workmux list' to see available worktrees.",
            name
        )
    })?;

    // Derive base handle from the worktree path (in case user provided branch name)
    let base_handle = worktree_path
        .file_name()
        .ok_or_else(|| anyhow!("Invalid worktree path: no directory name"))?
        .to_string_lossy()
        .to_string();

    // Resolve mode using canonical base_handle (not the CLI-provided name which may be a branch).
    // Precedence: CLI override > stored git metadata > config default (from options.mode)
    let stored_mode = git::get_worktree_mode_opt(&base_handle);
    let mode = mode_override.or(stored_mode).unwrap_or(options.mode);
    if mode == MuxMode::Session && options.window_session_name.is_some() {
        anyhow::bail!("--parent-session requires window mode");
    }
    let cli_target_window_name = options.target_window_name.clone();
    let cli_target_session_name = options.target_session_name.clone();
    let cli_window_session_name = options.window_session_name.clone();
    let explicit_primary_target_name = match mode {
        MuxMode::Window => cli_target_window_name.as_deref(),
        MuxMode::Session => cli_target_session_name.as_deref(),
    };

    if mode == MuxMode::Session && context.mux.name() != "tmux" {
        anyhow::bail!(
            "Session mode (--mode session / --session) is only supported with tmux.\n\
             Current backend: {}. Use window mode instead.",
            context.mux.name()
        );
    }

    // Validate windows config requires session mode (after canonical mode resolution)
    if let Some(windows) = &context.config.windows {
        if mode != MuxMode::Session {
            anyhow::bail!(
                "'windows' configuration requires 'mode: session'. \
                 Add 'mode: session' to your config."
            );
        }
        crate::config::validate_windows_config(windows)?;
    }

    let prior_mode = stored_mode.unwrap_or(MuxMode::Window);
    if let Some(explicit_target_name) = explicit_primary_target_name {
        let explicit_target = MuxHandle::new(
            context.mux.as_ref(),
            mode,
            &context.prefix,
            explicit_target_name,
        );
        if explicit_target.exists()? {
            let stored_target_name = match mode {
                MuxMode::Window => git::get_worktree_target_window(&base_handle),
                MuxMode::Session => git::get_worktree_target_session(&base_handle),
            };
            if stored_target_name.as_deref() != Some(explicit_target_name) {
                anyhow::bail!(
                    "A {} {} named '{}' already exists. Use a different target name.",
                    context.mux.name(),
                    explicit_target.kind(),
                    explicit_target.full_name()
                );
            }
        }
    }

    if prior_mode != mode {
        match prior_mode {
            MuxMode::Window => {
                // Kill all matching window targets (base + any -N numeric duplicates only)
                let all_names = context.mux.get_all_window_names()?;
                let prior_window_name = git::get_worktree_target_window(&base_handle)
                    .unwrap_or_else(|| base_handle.clone());
                let full_base = prefixed(&context.prefix, &prior_window_name);
                let full_base_dash = format!("{}-", full_base);
                for name in &all_names {
                    let is_exact = *name == full_base;
                    let is_numeric_suffix = name
                        .strip_prefix(&full_base_dash)
                        .is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()));

                    if is_exact || is_numeric_suffix {
                        info!(
                            handle = base_handle,
                            window = name,
                            "open:closing window before mode conversion"
                        );
                        MuxHandle::kill_full(context.mux.as_ref(), prior_mode, name)?;
                    }
                }
            }
            MuxMode::Session => {
                let prior_session_name = git::get_worktree_target_session(&base_handle)
                    .unwrap_or_else(|| base_handle.clone());
                let full_name = prefixed(&context.prefix, &prior_session_name);
                if MuxHandle::exists_full(context.mux.as_ref(), prior_mode, &full_name)? {
                    info!(
                        handle = base_handle,
                        session = full_name,
                        "open:closing session before mode conversion"
                    );
                    MuxHandle::kill_full(context.mux.as_ref(), prior_mode, &full_name)?;
                }
            }
        }
    }

    let target_window_name = options
        .target_window_name
        .clone()
        .or_else(|| git::get_worktree_target_window(&base_handle));
    let target_session_name = options
        .target_session_name
        .clone()
        .or_else(|| git::get_worktree_target_session(&base_handle));
    let window_session_name = options
        .window_session_name
        .clone()
        .or_else(|| git::get_worktree_window_session(&base_handle));

    // Update options with the resolved mode
    let options = SetupOptions {
        mode,
        target_window_name: if mode == MuxMode::Window {
            target_window_name.clone()
        } else {
            None
        },
        target_session_name: if mode == MuxMode::Session {
            target_session_name.clone()
        } else {
            None
        },
        window_session_name: if mode == MuxMode::Window {
            window_session_name.clone()
        } else {
            None
        },
        ..options
    };

    let target_name = match mode {
        MuxMode::Window => target_window_name.as_deref().unwrap_or(&base_handle),
        MuxMode::Session => target_session_name.as_deref().unwrap_or(&base_handle),
    };
    let target = MuxHandle::new(context.mux.as_ref(), mode, &context.prefix, target_name);
    let window_target = WindowTarget::new(target.full_name(), window_session_name.clone());
    let target_exists = if mode == MuxMode::Window {
        context.mux.window_target_exists(&window_target)?
    } else {
        target.exists()?
    };

    // If target exists and we're not forcing new, switch to it
    if target_exists && !new_window {
        // Backfill mode metadata for legacy worktrees on successful switch
        if stored_mode != Some(mode) {
            let mode_str = if mode == MuxMode::Session {
                "session"
            } else {
                "window"
            };
            let _ = git::set_worktree_meta(&base_handle, "mode", mode_str);
            if let Some(target_window_name) = &options.target_window_name {
                let _ = git::set_worktree_meta(&base_handle, "target-window", target_window_name);
            }
            if let Some(target_session_name) = &options.target_session_name {
                let _ = git::set_worktree_meta(&base_handle, "target-session", target_session_name);
            }
            if let Some(window_session_name) = &options.window_session_name {
                let _ = git::set_worktree_meta(&base_handle, "window-session", window_session_name);
            }
        }
        if options.focus_window {
            if mode == MuxMode::Window && window_target.parent_session().is_some() {
                context.mux.select_window_target(&window_target)?;
            } else {
                target.select()?;
            }
        }
        info!(
            handle = base_handle,
            branch = branch_name,
            path = %worktree_path.display(),
            kind = target.kind(),
            focus = options.focus_window,
            "open:switched to existing target"
        );
        return Ok(CreateResult {
            worktree_path,
            branch_name,
            post_create_hooks_run: 0,
            base_branch: None,
            did_switch: true,
            mux_target_full_name: target.full_name(),
            resolved_handle: base_handle,
            mode,
        });
    }

    // Session mode doesn't support --new (duplicate sessions would be orphaned on cleanup)
    if new_window && target.is_session() {
        return Err(anyhow!(
            "--new is not supported in session mode. Each worktree can only have one session."
        ));
    }

    // Persist mode metadata if it's missing or changing (backfill legacy worktrees).
    // Placed after early-exit checks to avoid side effects on failed commands.
    if stored_mode != Some(mode) {
        let mode_str = if mode == MuxMode::Session {
            "session"
        } else {
            "window"
        };
        git::set_worktree_meta(&base_handle, "mode", mode_str)
            .context("Failed to persist worktree mode")?;
        info!(
            handle = base_handle,
            mode = mode_str,
            "open:persisted worktree mode"
        );
    }
    if let Some(target_window_name) = &cli_target_window_name {
        git::set_worktree_meta(&base_handle, "target-window", target_window_name)
            .context("Failed to persist target window")?;
    }
    if mode == MuxMode::Session {
        if let Some(target_session_name) = &cli_target_session_name {
            git::set_worktree_meta(&base_handle, "target-session", target_session_name)
                .context("Failed to persist target session")?;
        }
    } else if let Some(window_session_name) = &cli_window_session_name {
        git::set_worktree_meta(&base_handle, "window-session", window_session_name)
            .context("Failed to persist window session")?;
    }

    // Determine handle: use suffix if forcing new target and one exists
    let (handle, after_window) = if new_window && target_exists {
        let unique_handle = resolve_unique_handle(context, target_name)?;
        // Insert after the last window in the target name group (base or -N suffixes)
        let after = context
            .mux
            .find_last_window_with_base_handle(&context.prefix, target_name)
            .unwrap_or(None);
        (unique_handle, after)
    } else {
        (target_name.to_string(), None)
    };

    // Compute working directory from config location
    let working_dir = if !context.config_rel_dir.as_os_str().is_empty() {
        let subdir_in_worktree = worktree_path.join(&context.config_rel_dir);
        if subdir_in_worktree.exists() {
            Some(subdir_in_worktree)
        } else {
            None
        }
    } else {
        None
    };

    // Use config_source_dir for file operations (the directory where config was found)
    let config_root = if !context.config_rel_dir.as_os_str().is_empty() {
        Some(context.config_source_dir.clone())
    } else {
        None
    };

    // In file-only mode, write prompt file to the worktree before pane setup
    // so editors/plugins can detect it on startup.
    if let Some(prompt) = prompt_file_only {
        setup::write_prompt_file(Some(&worktree_path), &branch_name, prompt)?;
    }

    let options_with_workdir = SetupOptions {
        working_dir,
        config_root,
        target_window_name: if mode == MuxMode::Window {
            Some(handle.clone())
        } else {
            None
        },
        ..options
    };

    // Setup the environment
    let result = setup::setup_environment(
        context.mux.as_ref(),
        &branch_name,
        &handle,
        &worktree_path,
        &context.config,
        &options_with_workdir,
        None,
        after_window,
    )?;
    info!(
        handle = handle,
        branch = branch_name,
        path = %result.worktree_path.display(),
        hooks_run = result.post_create_hooks_run,
        "open:completed"
    );
    Ok(result)
}

/// Find a unique handle by appending a suffix if necessary.
///
/// If `base_handle` is "my-feature" and windows exist for:
/// - wm-my-feature
/// - wm-my-feature-2
///
/// This returns "my-feature-3".
///
/// Note: Only called in window mode (session mode rejects --new).
fn resolve_unique_handle(context: &WorkflowContext, base_handle: &str) -> Result<String> {
    let all_names = context.mux.get_all_window_names()?;
    let prefix = &context.prefix;
    let full_base = prefixed(prefix, base_handle);

    // If base name doesn't exist, use it directly
    if !all_names.contains(&full_base) {
        return Ok(base_handle.to_string());
    }

    // Find the highest existing suffix
    // Pattern matches: {prefix}{handle}-{number}
    let escaped_base = regex::escape(&full_base);
    let pattern = format!(r"^{}-(\d+)$", escaped_base);
    let re = Regex::new(&pattern).expect("Invalid regex pattern");

    let mut max_suffix: u32 = 1; // Start at 1 so first duplicate is -2

    for name in &all_names {
        if let Some(caps) = re.captures(name)
            && let Some(num_match) = caps.get(1)
            && let Ok(num) = num_match.as_str().parse::<u32>()
        {
            max_suffix = max_suffix.max(num);
        }
    }

    let new_handle = format!("{}-{}", base_handle, max_suffix + 1);

    info!(
        base_handle = base_handle,
        new_handle = new_handle,
        "open:generated unique handle for duplicate"
    );

    Ok(new_handle)
}
