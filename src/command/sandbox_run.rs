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
    let mut lima_cmd = Command::new("limactl");
    lima_cmd.arg("shell").arg(&vm_name);

    // Pass through env vars from config
    for env_var in config.sandbox.env_passthrough() {
        if let Ok(val) = std::env::var(env_var) {
            lima_cmd.args(["--setenv", &format!("{}={}", env_var, val)]);
        }
    }

    // Set sandbox-specific env vars
    lima_cmd.args(["--setenv", "WM_SANDBOX_GUEST=1"]);
    lima_cmd.args(["--setenv", "WM_RPC_HOST=host.lima.internal"]);
    lima_cmd.args(["--setenv", &format!("WM_RPC_PORT={}", rpc_port)]);
    lima_cmd.args(["--setenv", &format!("WM_RPC_TOKEN={}", rpc_token)]);

    // Set working directory
    lima_cmd.args(["--workdir", &worktree.to_string_lossy()]);

    // Add the command separator and actual command
    lima_cmd.arg("--");
    for arg in &command {
        lima_cmd.arg(arg);
    }

    debug!(cmd = ?lima_cmd, "spawning limactl shell");

    // 6. Run the command (inherits stdin/stdout/stderr for interactive use)
    let status = lima_cmd
        .status()
        .context("Failed to execute limactl shell")?;

    let exit_code = status.code().unwrap_or(1);
    info!(exit_code, "agent command exited");

    Ok(exit_code)
}
