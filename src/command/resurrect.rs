use anyhow::{Result, anyhow};

use crate::config;
use crate::multiplexer::{create_backend, detect_backend};
use crate::state::StateStore;
use crate::workflow::resurrect::{ResurrectAction, plan};
use crate::workflow::{self, SetupOptions, WorkflowContext};

pub fn run(dry_run: bool) -> Result<()> {
    let config = config::Config::load(None)?;
    let mux = create_backend(detect_backend());
    let store = StateStore::new()?;

    let plan = plan(&store, mux.as_ref())?;

    if plan.candidates.is_empty() && plan.unmatched_states == 0 {
        println!("No agent state files found. Nothing to restore.");
        return Ok(());
    }

    // Print plan
    let to_restore: Vec<_> = plan
        .candidates
        .iter()
        .filter(|c| matches!(c.action, ResurrectAction::Restore))
        .collect();

    for candidate in &plan.candidates {
        let status = match &candidate.action {
            ResurrectAction::Restore => "restoring",
            ResurrectAction::SkipAlreadyOpen => "skipping (already open)",
            ResurrectAction::SkipMain => "skipping (main worktree)",
        };
        println!("  {:<20} -> {}", candidate.handle, status);
    }

    if plan.unmatched_states > 0 {
        println!(
            "  ({} unmatched state file(s) ignored)",
            plan.unmatched_states
        );
    }

    if to_restore.is_empty() {
        println!("\nNothing to restore.");
        return Ok(());
    }

    if dry_run {
        println!("\nDry run: would restore {} worktree(s)", to_restore.len());
        return Ok(());
    }

    // Execute restoration
    let context = WorkflowContext::new(config, mux, None)?;
    let mut restored = Vec::new();
    let mut failed = Vec::new();

    for candidate in &plan.candidates {
        if !matches!(candidate.action, ResurrectAction::Restore) {
            continue;
        }

        let options = SetupOptions {
            run_hooks: false,
            run_file_ops: false,
            run_pane_commands: true,
            prompt_file_path: None,
            focus_window: false,
            working_dir: None,
            config_root: None,
            open_if_exists: false,
            mode: candidate.mode,
            continue_session: true,
        };

        match workflow::open(&candidate.handle, &context, options, false, false, None) {
            Ok(_) => {
                // Clean up stale state files by specific PaneKey
                for key in &candidate.stale_pane_keys {
                    let _ = store.delete_agent(key);
                }
                restored.push(candidate.handle.clone());
            }
            Err(e) => {
                eprintln!("  Failed to restore '{}': {}", candidate.handle, e);
                failed.push(candidate.handle.clone());
            }
        }
    }

    // Summary
    if !restored.is_empty() {
        println!(
            "\n✓ Restored {} worktree(s): {}",
            restored.len(),
            restored.join(", ")
        );
    }
    if !failed.is_empty() {
        eprintln!(
            "✗ Failed to restore {} worktree(s): {}",
            failed.len(),
            failed.join(", ")
        );
        return Err(anyhow!("Failed to restore {} worktree(s)", failed.len()));
    }

    Ok(())
}
