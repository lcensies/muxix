//! Run a command in a worktree's tmux/wezterm window.

use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};

use crate::config::SplitDirection;
use crate::multiplexer::{create_backend, detect_backend};
use crate::state::run::{RunSpec, cleanup_run, create_run, generate_run_id, read_result};
use crate::workflow;

/// Escape a string for safe shell embedding.
fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./=@:".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub fn run(
    worktree_name: &str,
    command_parts: Vec<String>,
    wait: bool,
    keep: bool,
    timeout: Option<u64>,
) -> Result<()> {
    if command_parts.is_empty() {
        return Err(anyhow!("No command provided"));
    }

    let mux = create_backend(detect_backend());

    // Resolve worktree to agent pane (consistent with send/capture)
    let (worktree_path, agent) = workflow::resolve_worktree_agent(worktree_name, mux.as_ref())?;

    // Build command string (preserve argument boundaries via shell escaping)
    let command = command_parts
        .iter()
        .map(|s| shell_escape(s))
        .collect::<Vec<_>>()
        .join(" ");

    // Generate run ID and create spec
    let run_id = generate_run_id();
    let spec = RunSpec {
        command: command.clone(),
        worktree_path: worktree_path.clone(),
    };
    let run_dir = create_run(&run_id, &spec)?;

    // Get path to current executable for __exec
    let exe_path = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "workmux".to_string());

    // Split pane with __exec command (pass absolute run_dir path)
    let exec_cmd = format!(
        "{} __exec --run-dir {}",
        shell_escape(&exe_path),
        shell_escape(&run_dir.to_string_lossy())
    );
    let new_pane_id = mux.split_pane(
        &agent.pane_id,
        &SplitDirection::Vertical,
        &worktree_path,
        None,
        Some(30), // 30% for the command pane
        Some(&exec_cmd),
    )?;

    if !wait {
        eprintln!("Started: {} (run_id: {})", command, run_id);
        eprintln!("Pane: {}", new_pane_id);
        return Ok(());
    }

    // Poll for completion with optional timeout
    eprintln!("Running: {}", command);
    let start = Instant::now();
    let timeout_duration = timeout.map(Duration::from_secs);

    loop {
        // Check timeout
        if let Some(max_duration) = timeout_duration
            && start.elapsed() > max_duration
        {
            eprintln!("Timeout after {}s", timeout.unwrap());
            if !keep {
                let _ = cleanup_run(&run_dir);
            }
            std::process::exit(124); // Standard timeout exit code
        }

        if let Some(result) = read_result(&run_dir)? {
            // Read output files
            let stdout = std::fs::read_to_string(run_dir.join("stdout")).unwrap_or_default();
            let stderr = std::fs::read_to_string(run_dir.join("stderr")).unwrap_or_default();

            // Print captured output (already shown in pane, but useful if redirected)
            if !stdout.is_empty() {
                print!("{}", stdout);
            }
            if !stderr.is_empty() {
                eprint!("{}", stderr);
            }

            // Cleanup unless --keep
            if !keep {
                let _ = cleanup_run(&run_dir);
            }

            // Exit with command's exit code
            let exit_code = result.exit_code.unwrap_or(1);
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
}
