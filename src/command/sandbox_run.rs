//! The `workmux sandbox run` supervisor process.
//!
//! Runs inside a tmux pane. Starts a TCP RPC server and executes the agent
//! command inside a sandbox (Lima VM or Docker/Podman container).

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tracing::{debug, info, warn};

use std::collections::HashSet;

use crate::config::{Config, SandboxBackend, SandboxRuntime};
use crate::multiplexer;
use crate::sandbox::build_docker_run_args;
use crate::sandbox::ensure_sandbox_config_dirs;
use crate::sandbox::lima;
use crate::sandbox::rpc::{RpcContext, RpcServer, generate_token};
use crate::sandbox::shims;
use crate::sandbox::toolchain;
use crate::state::StateStore;

/// Guard that stops a container when dropped.
/// Ensures cleanup even if the supervisor is killed or panics.
struct ContainerGuard {
    runtime: &'static str,
    name: String,
    handle: String,
}

impl Drop for ContainerGuard {
    fn drop(&mut self) {
        debug!(container = %self.name, "stopping container");
        let result = Command::new(self.runtime)
            .args(["stop", "-t", "2", &self.name])
            .output();
        match result {
            Ok(output) if output.status.success() => {
                debug!(container = %self.name, "container stopped");
            }
            Ok(output) => {
                // Container may have already exited, which is fine
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.contains("No such container") {
                    warn!(container = %self.name, stderr = %stderr.trim(), "failed to stop container");
                }
            }
            Err(e) => {
                warn!(container = %self.name, error = %e, "failed to run docker stop");
            }
        }

        // Unregister container from state store
        if let Ok(store) = StateStore::new() {
            store.unregister_container(&self.handle, &self.name);
        }
    }
}

/// Run the sandbox supervisor.
///
/// Detects the sandbox backend from config and dispatches to the
/// appropriate handler (Lima VM or Docker/Podman container).
pub fn run(worktree: PathBuf, worktree_root: Option<PathBuf>, command: Vec<String>) -> Result<i32> {
    if command.is_empty() {
        bail!("No command specified. Usage: workmux sandbox run <worktree> -- <command...>");
    }

    let config = Config::load(None)?;
    let worktree = worktree.canonicalize().unwrap_or_else(|_| worktree.clone());

    match config.sandbox.backend() {
        SandboxBackend::Lima => run_lima(&config, &worktree, &command),
        SandboxBackend::Container => {
            let wt_root = worktree_root
                .map(|p| p.canonicalize().unwrap_or(p))
                .unwrap_or_else(|| worktree.clone());
            run_container(&config, &worktree, &wt_root, &command)
        }
    }
}

/// Start RPC server and return (server, port, token, context).
/// Shared setup between Lima and Container backends.
fn start_rpc(
    worktree: &Path,
    allowed_commands: HashSet<String>,
    detected_toolchain: toolchain::DetectedToolchain,
) -> Result<(RpcServer, u16, String, Arc<RpcContext>)> {
    let rpc_server = RpcServer::bind()?;
    let rpc_port = rpc_server.port();
    let rpc_token = generate_token();
    info!(port = rpc_port, "RPC server listening");

    let mux = multiplexer::create_backend(multiplexer::detect_backend());
    let pane_id = mux.current_pane_id().unwrap_or_default();

    let ctx = Arc::new(RpcContext {
        pane_id,
        worktree_path: worktree.to_path_buf(),
        mux,
        token: rpc_token.clone(),
        allowed_commands,
        detected_toolchain,
    });

    Ok((rpc_server, rpc_port, rpc_token, ctx))
}

fn run_lima(config: &Config, worktree: &Path, command: &[String]) -> Result<i32> {
    info!(worktree = %worktree.display(), "sandbox supervisor starting (lima)");

    // Ensure Lima VM is running
    let vm_name = lima::ensure_vm_running(config, worktree)?;
    info!(vm_name = %vm_name, "Lima VM ready");

    if let Err(e) = lima::mounts::seed_claude_json(&vm_name) {
        tracing::warn!(vm_name = %vm_name, error = %e, "failed to seed ~/.claude.json; continuing");
    }

    // Detect toolchain for both agent wrapping and host-exec
    let detected = toolchain::resolve_toolchain(&config.sandbox.toolchain(), worktree);
    if detected != toolchain::DetectedToolchain::None {
        info!(toolchain = ?detected, "wrapping command with toolchain environment");
    }

    // Create host-exec shims for configured commands
    let host_commands = config.sandbox.host_commands();
    let allowed_commands: HashSet<String> = host_commands.iter().cloned().collect();

    if !host_commands.is_empty() {
        let state_dir = lima::mounts::lima_state_dir_path(&vm_name)?;
        shims::create_shim_directory(&state_dir, host_commands)?;
        info!(commands = ?host_commands, "created host-exec shims");
    }

    let (rpc_server, rpc_port, rpc_token, ctx) =
        start_rpc(worktree, allowed_commands, detected.clone())?;
    let _rpc_handle = rpc_server.spawn(ctx);

    // Build limactl shell command
    let mut lima_cmd = Command::new("limactl");
    lima_cmd
        .arg("shell")
        .args(["--workdir", &worktree.to_string_lossy()])
        .arg(&vm_name);

    let mut env_exports = vec![
        r#"PATH="$HOME/.workmux-state/shims/bin:$HOME/.local/bin:/nix/var/nix/profiles/default/bin:$PATH""#.to_string(),
        "WM_SANDBOX_GUEST=1".to_string(),
        "WM_RPC_HOST=host.lima.internal".to_string(),
        format!("WM_RPC_PORT={}", rpc_port),
        format!("WM_RPC_TOKEN={}", rpc_token),
    ];

    for term_var in ["TERM", "COLORTERM"] {
        if let Ok(val) = std::env::var(term_var) {
            env_exports.push(format!("{}={}", term_var, val));
        }
    }

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

    let final_command = toolchain::wrap_command(&user_command, &detected);
    let full_command = format!("{exports}; {final_command}");

    lima_cmd.arg("--");
    lima_cmd.arg("eval");
    lima_cmd.arg(&full_command);

    debug!(cmd = ?lima_cmd, "spawning limactl shell");

    let status = lima_cmd
        .status()
        .context("Failed to execute limactl shell")?;

    let exit_code = status.code().unwrap_or(1);
    info!(exit_code, "agent command exited");
    Ok(exit_code)
}

fn run_container(
    config: &Config,
    pane_cwd: &Path,
    worktree_root: &Path,
    command: &[String],
) -> Result<i32> {
    info!(
        pane_cwd = %pane_cwd.display(),
        worktree_root = %worktree_root.display(),
        "sandbox supervisor starting (container)"
    );

    // Validate that pane_cwd is under worktree_root
    if !pane_cwd.starts_with(worktree_root) {
        bail!(
            "Working directory {} is not under worktree root {}",
            pane_cwd.display(),
            worktree_root.display()
        );
    }

    // Ensure sandbox config dirs exist before building container args
    ensure_sandbox_config_dirs()?;

    let host_commands = config.sandbox.host_commands();
    let allowed_commands: HashSet<String> = host_commands.iter().cloned().collect();

    let detected = toolchain::resolve_toolchain(&config.sandbox.toolchain(), worktree_root);
    if detected != toolchain::DetectedToolchain::None {
        info!(toolchain = ?detected, "wrapping commands with toolchain environment");
    }

    // Create shims directory for host-exec (on host, will be bind-mounted into container)
    let _shim_dir = if !host_commands.is_empty() {
        let dir = tempfile::tempdir().context("Failed to create shim temp dir")?;
        shims::create_shim_directory(dir.path(), host_commands)?;
        info!(commands = ?host_commands, "created host-exec shims");
        Some(dir)
    } else {
        None
    };

    let (rpc_server, rpc_port, rpc_token, ctx) =
        start_rpc(pane_cwd, allowed_commands, detected.clone())?;
    let _rpc_handle = rpc_server.spawn(ctx);

    // Compute RPC host BEFORE matching on runtime (SandboxRuntime is not Copy)
    let rpc_host = config.sandbox.resolved_rpc_host();
    let runtime = config.sandbox.runtime();
    let runtime_bin: &'static str = match runtime {
        SandboxRuntime::Podman => "podman",
        SandboxRuntime::Docker => "docker",
    };

    // Generate container name from worktree directory name so cleanup can find it.
    // Include PID to allow multiple agents in the same worktree (e.g., open -n).
    let handle = worktree_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();
    let container_name = format!("wm-{}-{}", handle, std::process::id());

    // Register container in state store so cleanup can find it without docker ps
    if let Ok(store) = StateStore::new()
        && let Err(e) = store.register_container(&handle, &container_name)
    {
        warn!(error = %e, "failed to register container state");
    }

    let rpc_port_str = rpc_port.to_string();
    let extra_envs = [
        ("WM_SANDBOX_GUEST", "1"),
        ("WM_RPC_HOST", rpc_host.as_str()),
        ("WM_RPC_PORT", rpc_port_str.as_str()),
        ("WM_RPC_TOKEN", rpc_token.as_str()),
    ];

    let user_command = command.join(" ");
    let final_command = toolchain::wrap_command(&user_command, &detected);
    let shim_host_dir = _shim_dir.as_ref().map(|d| d.path().join("shims/bin"));
    let mut docker_args = build_docker_run_args(
        &final_command,
        &config.sandbox,
        worktree_root,
        pane_cwd,
        &extra_envs,
        shim_host_dir.as_deref(),
    )?;

    // Insert --name after "run" (index 0 is "run")
    docker_args.insert(1, "--name".to_string());
    docker_args.insert(2, container_name.clone());

    debug!(runtime = runtime_bin, container = %container_name, args = ?docker_args, "spawning container");

    // Create guard to stop container on exit (panic, SIGTERM, etc.)
    let _guard = ContainerGuard {
        runtime: runtime_bin,
        name: container_name,
        handle,
    };

    let status = Command::new(runtime_bin)
        .args(&docker_args)
        .status()
        .with_context(|| format!("Failed to execute {} run", runtime_bin))?;

    let exit_code = status.code().unwrap_or(1);
    info!(exit_code, "container command exited");
    Ok(exit_code)
}
