//! The `workmux sandbox run` supervisor process.
//!
//! Runs inside a tmux pane. Manages the Lima VM, starts a TCP RPC server,
//! and executes the agent command inside the VM via `limactl shell`.

use anyhow::{Context, Result, bail};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tracing::{debug, info};

use crate::config::Config;
use crate::multiplexer;
use crate::sandbox::lima;
use crate::sandbox::rpc::{RpcContext, RpcServer, generate_token};

/// Run the sandbox supervisor.
///
/// This is the long-lived process that runs in a tmux pane:
/// 1. Ensures the Lima VM is running
/// 2. Starts the TCP RPC server on a random port
/// 3. Executes the agent command inside the VM via `limactl shell`
/// 4. Returns the agent's exit code
pub fn run(worktree: PathBuf, command: Vec<String>) -> Result<i32> {
    if command.is_empty() {
        bail!("No command specified. Usage: workmux sandbox run <worktree> -- <command...>");
    }

    let config = Config::load(None)?;
    let worktree = worktree.canonicalize().unwrap_or_else(|_| worktree.clone());

    info!(worktree = %worktree.display(), "sandbox supervisor starting");

    // 1. Ensure Lima VM is running (idempotent -- fast if already booted)
    let vm_name = lima::ensure_vm_running(&config, &worktree)?;
    info!(vm_name = %vm_name, "Lima VM ready");

    // 2. Start RPC server
    let rpc_server = RpcServer::bind()?;
    let rpc_port = rpc_server.port();
    let rpc_token = generate_token();
    info!(port = rpc_port, "RPC server listening");

    // 3. Resolve multiplexer backend and pane ID
    let mux = multiplexer::create_backend(multiplexer::detect_backend());
    let pane_id = mux.current_pane_id().unwrap_or_default();

    let ctx = Arc::new(RpcContext {
        pane_id,
        worktree_path: worktree.clone(),
        mux,
        token: rpc_token.clone(),
    });

    // 4. Spawn RPC acceptor thread
    let _rpc_handle = rpc_server.spawn(ctx);

    // 5. Build limactl shell command
    //
    // Important: `limactl shell` uses cobra with non-interspersed args, so
    // all flags (--workdir) must come BEFORE the instance name. Anything
    // after the instance name is treated as the remote command.
    //
    // Also, `limactl shell` does NOT support `--setenv`. Environment
    // variables are passed by embedding `export` statements in the command.
    let mut lima_cmd = Command::new("limactl");
    lima_cmd
        .arg("shell")
        .args(["--workdir", &worktree.to_string_lossy()])
        .arg(&vm_name);

    // Build env var exports to embed in the command.
    // limactl shell wraps commands in `$SHELL --login -c '<escaped>'` where
    // each arg is individually shell-quoted, then the joined string is quoted
    // AGAIN for -c. This double-quoting prevents shell expansion (e.g.,
    // $(cat ...) would become literal). Prepending `eval` adds one extra
    // level of shell interpretation that undoes Lima's protective quoting.
    let mut env_exports = vec![
        // Ensure ~/.local/bin is on PATH (Claude Code installs there)
        r#"PATH="$HOME/.local/bin:$PATH""#.to_string(),
        "WM_SANDBOX_GUEST=1".to_string(),
        "WM_RPC_HOST=host.lima.internal".to_string(),
        format!("WM_RPC_PORT={}", rpc_port),
        format!("WM_RPC_TOKEN={}", rpc_token),
    ];
    for env_var in config.sandbox.env_passthrough() {
        if let Ok(val) = std::env::var(env_var) {
            env_exports.push(format!("{}={}", env_var, val));
        }
    }

    let exports: String = env_exports
        .iter()
        .map(|e| format!("export {e}"))
        .collect::<Vec<_>>()
        .join("; ");
    let user_command = command.join(" ");
    let full_command = format!("{exports}; {user_command}");

    lima_cmd.arg("--");
    lima_cmd.arg("eval");
    lima_cmd.arg(&full_command);

    debug!(cmd = ?lima_cmd, "spawning limactl shell");

    // 6. Run the command (inherits stdin/stdout/stderr for interactive use)
    let status = lima_cmd
        .status()
        .context("Failed to execute limactl shell")?;

    let exit_code = status.code().unwrap_or(1);
    info!(exit_code, "agent command exited");

    Ok(exit_code)
}
