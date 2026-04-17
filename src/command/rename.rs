use anyhow::{Result, anyhow};

use crate::config;
use crate::multiplexer::{create_backend, detect_backend};
use crate::workflow::{self, WorkflowContext};

pub fn run(names: Vec<String>, rename_branch: bool) -> Result<()> {
    // Capture the caller's CWD before any chdir operations, so we can warn
    // accurately at the end if it's rooted inside the worktree being moved.
    let original_cwd = std::env::current_dir().ok();

    let (old_name, new_name) = match names.len() {
        1 => (super::resolve_name(None)?, names[0].clone()),
        2 => (super::resolve_name(Some(&names[0]))?, names[1].clone()),
        _ => return Err(anyhow!("Expected 1 or 2 positional arguments")),
    };

    let config = config::Config::load(None)?;
    let mux = create_backend(detect_backend());
    let context = WorkflowContext::new(config, mux, None)?;

    let result = workflow::rename(&old_name, &new_name, rename_branch, &context)?;

    if result.old_handle != result.new_handle {
        println!(
            "✓ Renamed worktree '{}' -> '{}'",
            result.old_handle, result.new_handle
        );
    }

    if let Some(ref new_branch) = result.new_branch
        && new_branch != &result.old_branch
    {
        println!(
            "✓ Renamed branch '{}' -> '{}'",
            result.old_branch, new_branch
        );
    }

    if result.tmux_renamed > 0 {
        let noun = if result.tmux_renamed == 1 {
            "target"
        } else {
            "targets"
        };
        println!("✓ Renamed {} tmux {}", result.tmux_renamed, noun);
    }

    if result.agents_migrated > 0 {
        let noun = if result.agents_migrated == 1 {
            "file"
        } else {
            "files"
        };
        println!("✓ Updated {} agent state {}", result.agents_migrated, noun);
    }

    // Warn if the caller's shell is still rooted inside the old (now moved) path.
    if let Some(cwd) = original_cwd
        && cwd.starts_with(&result.old_path)
    {
        eprintln!(
            "\nNote: your shell's working directory is stale. Run \
             'cd {}' (or 'cd .') to refresh.",
            result.new_path.display()
        );
    }

    Ok(())
}
