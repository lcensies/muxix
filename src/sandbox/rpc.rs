//! TCP RPC protocol for guest-host communication in sandboxed environments.
//!
//! The host-side supervisor runs an RPC server on a random port. The guest
//! workmux binary connects via a host-internal address and sends JSON-lines
//! requests.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::multiplexer::{AgentStatus, Multiplexer};

// ── Protocol types ──────────────────────────────────────────────────────

/// RPC request sent from guest to host.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RpcRequest {
    SetStatus {
        status: String,
    },
    SetTitle {
        title: String,
    },
    Heartbeat,
    SpawnAgent {
        prompt: String,
        branch_name: Option<String>,
        background: Option<bool>,
    },
    Exec {
        command: String,
        args: Vec<String>,
    },
    Merge {
        name: String,
        into: Option<String>,
        rebase: bool,
        squash: bool,
        ignore_uncommitted: bool,
        keep: bool,
        no_verify: bool,
        no_hooks: bool,
        notification: bool,
    },
}

/// RPC response sent from host to guest.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RpcResponse {
    Ok,
    Error { message: String },
    Output { message: String },
    ExecOutput { data: String },
    ExecError { data: String },
    ExecExit { code: i32 },
}

// ── Server ──────────────────────────────────────────────────────────────

/// Context available to RPC request handlers.
pub struct RpcContext {
    /// The tmux/wezterm pane ID of the supervisor pane.
    pub pane_id: String,
    /// Path to the worktree being supervised.
    pub worktree_path: PathBuf,
    /// Multiplexer backend (resolved once at startup).
    pub mux: Arc<dyn Multiplexer>,
    /// Shared secret for authenticating RPC requests.
    pub token: String,
    /// Commands allowed for host-exec.
    pub allowed_commands: std::collections::HashSet<String>,
    /// Resolved toolchain for host-exec command wrapping.
    pub detected_toolchain: crate::sandbox::toolchain::DetectedToolchain,
}

/// TCP RPC server that accepts guest connections.
pub struct RpcServer {
    listener: TcpListener,
    port: u16,
}

impl RpcServer {
    /// Bind to a random port on all interfaces.
    ///
    /// Must bind to `0.0.0.0` (not `127.0.0.1`) because the Lima VM connects
    /// via `host.lima.internal`, which resolves to the host's gateway IP on
    /// the shared network interface, not the loopback address.
    pub fn bind() -> Result<Self> {
        let listener = TcpListener::bind("0.0.0.0:0").context("Failed to bind RPC listener")?;
        let port = listener.local_addr()?.port();
        info!(port, "RPC server bound");
        Ok(Self { listener, port })
    }

    /// Get the port the server is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Spawn a background thread that accepts connections and dispatches handlers.
    pub fn spawn(self, ctx: Arc<RpcContext>) -> thread::JoinHandle<()> {
        /// Max concurrent RPC connections. One sandbox session typically uses a
        /// single connection, so 16 is generous while still preventing thread
        /// exhaustion from malicious connection floods.
        const MAX_CONNECTIONS: usize = 16;

        let active = Arc::new(AtomicUsize::new(0));
        thread::spawn(move || {
            for stream in self.listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let current = active.load(Ordering::Relaxed);
                        if current >= MAX_CONNECTIONS {
                            warn!(current, "RPC connection limit reached, dropping");
                            drop(stream);
                            continue;
                        }
                        active.fetch_add(1, Ordering::Relaxed);
                        let ctx = Arc::clone(&ctx);
                        let active = Arc::clone(&active);
                        thread::spawn(move || {
                            if let Err(e) = handle_connection(stream, &ctx) {
                                debug!(error = %e, "RPC connection ended");
                            }
                            active.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(e) => {
                        debug!(error = %e, "RPC accept error, shutting down");
                        break;
                    }
                }
            }
        })
    }
}

/// Generate a random token for RPC authentication.
pub fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("failed to get random bytes");
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Constant-time byte comparison to prevent timing side-channel attacks.
/// Always compares every byte regardless of where the first difference is.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ── Connection handler ──────────────────────────────────────────────────

/// Maximum size of a single RPC request line (1 MB).
/// Prevents memory exhaustion from a malicious guest sending unbounded data.
const MAX_REQUEST_LINE: usize = 1024 * 1024;

/// Header line sent by client before requests. Contains the auth token.
#[derive(Debug, Serialize, Deserialize)]
struct AuthHeader {
    token: String,
}

/// Read a single line from a buffered reader, enforcing a size limit.
/// Returns `Ok(None)` on EOF, `Err` if the line exceeds the limit.
///
/// Accumulates raw bytes first, then validates UTF-8 once the line is
/// complete. This avoids false rejections when multi-byte UTF-8 characters
/// are split across buffer boundaries.
fn read_bounded_line(reader: &mut impl BufRead, buf: &mut String) -> Result<Option<()>> {
    buf.clear();
    let mut bytes = Vec::new();
    let mut total = 0usize;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if total == 0 {
                return Ok(None);
            }
            break;
        }

        let (take, done) = match available.iter().position(|&b| b == b'\n') {
            Some(pos) => (pos + 1, true),
            None => (available.len(), false),
        };

        total += take;
        if total > MAX_REQUEST_LINE {
            anyhow::bail!("RPC request line exceeds {} byte limit", MAX_REQUEST_LINE);
        }

        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);

        if done {
            break;
        }
    }

    let s = std::str::from_utf8(&bytes).context("Invalid UTF-8 in RPC request")?;
    buf.push_str(s);
    Ok(Some(()))
}

fn handle_connection(stream: TcpStream, ctx: &RpcContext) -> Result<()> {
    let peer = stream.peer_addr().ok();
    debug!(?peer, "RPC connection accepted");

    // Require auth header within 5 seconds to prevent slowloris-style DoS.
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;

    let mut reader = BufReader::new(&stream);
    let mut writer = stream.try_clone().context("Failed to clone TCP stream")?;

    // First line must be auth header (bounded read)
    let mut auth_line = String::new();
    match read_bounded_line(&mut reader, &mut auth_line)? {
        Some(()) => {}
        None => return Ok(()),
    }
    let auth: AuthHeader =
        serde_json::from_str(auth_line.trim()).context("Failed to parse auth header")?;

    if !constant_time_eq(auth.token.as_bytes(), ctx.token.as_bytes()) {
        let resp = RpcResponse::Error {
            message: "Invalid token".to_string(),
        };
        write_response(&mut writer, &resp)?;
        return Ok(());
    }

    // Clear timeout for authenticated connections so long-running requests
    // (e.g., Exec streaming) are not interrupted.
    stream.set_read_timeout(None)?;

    // Process request lines (bounded reads)
    let mut line = String::new();
    loop {
        match read_bounded_line(&mut reader, &mut line)? {
            Some(()) => {}
            None => break,
        }

        if line.trim().is_empty() {
            continue;
        }

        let request: RpcRequest = serde_json::from_str(line.trim())
            .with_context(|| format!("Failed to parse RPC request: {}", line.trim()))?;

        info!(?request, "RPC request received");

        // Exec and Merge require streaming multiple responses, handle separately
        if let RpcRequest::Exec {
            ref command,
            ref args,
        } = request
        {
            handle_exec(command, args, ctx, &mut writer)?;
            continue;
        }

        if let RpcRequest::Merge {
            ref name,
            ref into,
            rebase,
            squash,
            ignore_uncommitted,
            keep,
            no_verify: _,
            no_hooks: _,
            notification,
        } = request
        {
            // SECURITY: Force --no-verify --no-hooks regardless of guest request.
            // Hooks are user-configured shell commands that run unsandboxed on the
            // host. A compromised guest could modify .workmux.yaml to inject
            // malicious hooks, then trigger them via this RPC.
            handle_merge(
                name,
                into.as_deref(),
                rebase,
                squash,
                ignore_uncommitted,
                keep,
                notification,
                &ctx.worktree_path,
                &mut writer,
            )?;
            continue;
        }

        let response = dispatch_request(&request, ctx);
        debug!(?response, "RPC response");

        write_response(&mut writer, &response)?;
    }

    Ok(())
}

fn write_response(writer: &mut impl Write, response: &RpcResponse) -> Result<()> {
    let mut json = serde_json::to_string(response)?;
    json.push('\n');
    writer.write_all(json.as_bytes())?;
    writer.flush()?;
    Ok(())
}

// ── Request dispatch ────────────────────────────────────────────────────

fn dispatch_request(request: &RpcRequest, ctx: &RpcContext) -> RpcResponse {
    match request {
        RpcRequest::Heartbeat => RpcResponse::Ok,
        RpcRequest::SetStatus { status } => handle_set_status(status, ctx),
        RpcRequest::SetTitle { title } => handle_set_title(title, ctx),
        RpcRequest::SpawnAgent {
            prompt,
            branch_name,
            background,
        } => handle_spawn_agent(
            prompt,
            branch_name.as_deref(),
            *background,
            &ctx.worktree_path,
        ),
        RpcRequest::Exec { .. } => {
            // Handled in handle_connection before dispatch
            unreachable!("Exec is handled directly in handle_connection")
        }
        RpcRequest::Merge { .. } => {
            // Handled in handle_connection before dispatch (needs streaming)
            unreachable!("Merge is handled directly in handle_connection")
        }
    }
}

// ── Handlers ────────────────────────────────────────────────────────────

fn handle_set_status(status: &str, ctx: &RpcContext) -> RpcResponse {
    // Reuse the same logic as set_window_status command
    let config = match Config::load(None) {
        Ok(c) => c,
        Err(e) => {
            return RpcResponse::Error {
                message: format!("Failed to load config: {}", e),
            };
        }
    };

    let (agent_status, icon, auto_clear) = match status.to_lowercase().as_str() {
        "working" => (
            Some(AgentStatus::Working),
            config.status_icons.working().to_string(),
            false,
        ),
        "waiting" => (
            Some(AgentStatus::Waiting),
            config.status_icons.waiting().to_string(),
            true,
        ),
        "done" => (
            Some(AgentStatus::Done),
            config.status_icons.done().to_string(),
            true,
        ),
        "clear" => {
            if let Err(e) = ctx.mux.clear_status(&ctx.pane_id) {
                return RpcResponse::Error {
                    message: format!("Failed to clear status: {}", e),
                };
            }
            return RpcResponse::Ok;
        }
        _ => {
            return RpcResponse::Error {
                message: format!("Unknown status: {}", status),
            };
        }
    };

    if config.status_format.unwrap_or(true) {
        let _ = ctx.mux.ensure_status_format(&ctx.pane_id);
    }

    match ctx.mux.set_status(&ctx.pane_id, &icon, auto_clear) {
        Ok(()) => {
            // Persist agent state to StateStore so the dashboard sees this agent
            if let Some(agent_status) = agent_status {
                crate::state::persist_agent_update(
                    &*ctx.mux,
                    &ctx.pane_id,
                    Some(agent_status),
                    None,
                );
            }
            RpcResponse::Ok
        }
        Err(e) => RpcResponse::Error {
            message: format!("Failed to set status: {}", e),
        },
    }
}

fn handle_set_title(title: &str, ctx: &RpcContext) -> RpcResponse {
    // Use tmux rename-window via the Cmd helper (consistent with codebase patterns)
    use crate::cmd::Cmd;

    match Cmd::new("tmux")
        .args(&["rename-window", "-t", &ctx.pane_id, title])
        .run()
    {
        Ok(_) => {
            // Persist title to StateStore so the dashboard sees it
            crate::state::persist_agent_update(
                &*ctx.mux,
                &ctx.pane_id,
                None,
                Some(title.to_string()),
            );
            RpcResponse::Ok
        }
        Err(e) => RpcResponse::Error {
            message: format!("Failed to set title: {}", e),
        },
    }
}

fn handle_spawn_agent(
    prompt: &str,
    branch_name: Option<&str>,
    _background: Option<bool>,
    worktree_path: &PathBuf,
) -> RpcResponse {
    use std::process::Command;

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("workmux"));
    let mut cmd = Command::new(exe);
    cmd.arg("add");

    if let Some(name) = branch_name {
        cmd.arg(name);
    } else {
        cmd.arg("--auto-name");
    }

    cmd.args(["--prompt", prompt]);

    // SECURITY: Skip post-create hooks when triggered via RPC. Hooks are
    // arbitrary shell commands from config that run unsandboxed on the host.
    cmd.arg("--no-hooks");

    // Run from the worktree directory so config is found
    cmd.current_dir(worktree_path);

    match cmd.output() {
        Ok(output) if output.status.success() => RpcResponse::Ok,
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            RpcResponse::Error {
                message: format!("workmux add failed: {}", stderr.trim()),
            }
        }
        Err(e) => RpcResponse::Error {
            message: format!("Failed to run workmux add: {}", e),
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_merge(
    name: &str,
    into: Option<&str>,
    rebase: bool,
    squash: bool,
    ignore_uncommitted: bool,
    keep: bool,
    notification: bool,
    worktree_path: &PathBuf,
    writer: &mut impl Write,
) -> Result<()> {
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("workmux"));
    let mut cmd = Command::new(exe);
    cmd.arg("merge");
    cmd.arg(name);

    if let Some(target) = into {
        cmd.args(["--into", target]);
    }
    if rebase {
        cmd.arg("--rebase");
    }
    if squash {
        cmd.arg("--squash");
    }
    if ignore_uncommitted {
        cmd.arg("--ignore-uncommitted");
    }
    if keep {
        cmd.arg("--keep");
    }
    if notification {
        cmd.arg("--notification");
    }

    // SECURITY: Always skip hooks when triggered via RPC. Hooks are arbitrary
    // shell commands from config that run unsandboxed on the host.
    cmd.args(["--no-verify", "--no-hooks"]);

    // Run from the worktree directory so config is found
    cmd.current_dir(worktree_path);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            write_response(
                writer,
                &RpcResponse::Error {
                    message: format!("Failed to run workmux merge: {}", e),
                },
            )?;
            return Ok(());
        }
    };

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Stream stdout and stderr as Output responses
    let (tx, rx) = std::sync::mpsc::channel::<String>();

    let tx_out = tx.clone();
    let stdout_thread = thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        let mut reader = std::io::BufReader::new(stdout);
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let _ = tx_out.send(data);
                }
                Err(_) => break,
            }
        }
    });

    let tx_err = tx.clone();
    let stderr_thread = thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        let mut reader = std::io::BufReader::new(stderr);
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let _ = tx_err.send(data);
                }
                Err(_) => break,
            }
        }
    });

    drop(tx);

    // Stream responses; kill child on write failure (mirrors handle_exec pattern)
    let stream_result = (|| -> Result<()> {
        for chunk in rx {
            write_response(writer, &RpcResponse::Output { message: chunk })?;
        }
        Ok(())
    })();

    if stream_result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return stream_result;
    }

    stdout_thread.join().ok();
    stderr_thread.join().ok();

    let status = child.wait()?;
    if status.success() {
        write_response(writer, &RpcResponse::Ok)?;
    } else {
        write_response(
            writer,
            &RpcResponse::Error {
                message: format!(
                    "workmux merge exited with code {}",
                    status.code().unwrap_or(1)
                ),
            },
        )?;
    }

    Ok(())
}

/// Environment variables allowed to pass through to host-exec child processes.
/// Everything else is cleared to prevent leaking host secrets.
const EXEC_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TERM",
    "COLORTERM",
    "LANG",
    "LC_ALL",
];

/// Build a sanitized environment map from the current process environment.
/// Only variables in the allowlist are included. PATH is normalized to
/// contain only absolute entries (prevents relative-path hijacking from
/// the worktree's current directory).
fn sanitized_env() -> std::collections::HashMap<String, String> {
    let mut envs = std::collections::HashMap::new();
    for key in EXEC_ENV_ALLOWLIST {
        if let Ok(val) = std::env::var(key) {
            if *key == "PATH" {
                // Strip relative/empty PATH entries to prevent hijacking
                let normalized: String = val
                    .split(':')
                    .filter(|p| p.starts_with('/'))
                    .collect::<Vec<_>>()
                    .join(":");
                envs.insert(key.to_string(), normalized);
            } else {
                envs.insert(key.to_string(), val);
            }
        }
    }
    envs
}

fn handle_exec(
    command: &str,
    args: &[String],
    ctx: &RpcContext,
    writer: &mut impl Write,
) -> Result<()> {
    info!(command, ?args, "host-exec request");

    // Validate command name format (strict alphanumeric + dash/underscore/dot)
    if !crate::sandbox::shims::validate_command_name(command) {
        let resp = RpcResponse::ExecExit { code: 127 };
        write_response(writer, &resp)?;
        return Ok(());
    }

    // Validate command is in allowlist
    if !ctx.allowed_commands.contains(command) {
        let resp = RpcResponse::ExecExit { code: 127 };
        write_response(writer, &resp)?;
        return Ok(());
    }

    // Skip toolchain wrapping for built-in host commands (e.g., afplay) since they
    // exist outside the project's devbox/nix environment
    let is_builtin = crate::sandbox::shims::BUILTIN_HOST_COMMANDS.contains(&command);
    let wrapper_script = if !is_builtin {
        crate::sandbox::toolchain::toolchain_wrapper_script(&ctx.detected_toolchain)
    } else {
        None
    };

    // Build the logical command (program + args), then delegate to sandbox
    let (program, final_args) = if let Some(script) = wrapper_script {
        // Safe toolchain wrapping: command and args are passed as positional
        // parameters to bash, never interpolated into the shell string.
        // bash -c '<script>' -- <command> <arg1> <arg2> ...
        let mut script_args = vec![
            "-c".to_string(),
            script,
            "--".to_string(),
            command.to_string(),
        ];
        script_args.extend_from_slice(args);
        ("bash".to_string(), script_args)
    } else {
        // Direct execution: no shell involved, args passed as argv
        (command.to_string(), args.to_vec())
    };

    let envs = sanitized_env();
    let spawn_result = crate::sandbox::host_exec_sandbox::spawn_sandboxed(
        &program,
        &final_args,
        &ctx.worktree_path,
        &envs,
    );

    let mut child = match spawn_result {
        Ok(child) => child,
        Err(e) => {
            warn!(command, error = %e, "failed to spawn command");
            write_response(writer, &RpcResponse::ExecExit { code: 127 })?;
            return Ok(());
        }
    };

    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();

    // Read stdout and stderr in threads, collect chunks
    let (tx, rx) = std::sync::mpsc::channel::<RpcResponse>();

    let tx_out = tx.clone();
    let stdout_thread = thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let _ = tx_out.send(RpcResponse::ExecOutput { data });
                }
                Err(_) => break,
            }
        }
    });

    let tx_err = tx.clone();
    let stderr_thread = thread::spawn(move || {
        use std::io::Read;
        let mut buf = [0u8; 8192];
        loop {
            match stderr.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).into_owned();
                    let _ = tx_err.send(RpcResponse::ExecError { data });
                }
                Err(_) => break,
            }
        }
    });

    // Drop our sender so rx closes when threads finish
    drop(tx);

    // Stream responses as they arrive; kill child on write failure
    let stream_result = (|| -> Result<()> {
        for response in rx {
            write_response(writer, &response)?;
        }
        Ok(())
    })();

    if stream_result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return stream_result;
    }

    stdout_thread.join().ok();
    stderr_thread.join().ok();

    let status = child.wait()?;
    let code = status.code().unwrap_or(1);
    info!(command, code, "host-exec finished");

    write_response(writer, &RpcResponse::ExecExit { code })?;
    Ok(())
}

// ── Client ──────────────────────────────────────────────────────────────

/// RPC client for guest-side use. Connects to the host supervisor.
///
/// Used by the guest workmux binary to send requests to the host supervisor
/// when `WM_SANDBOX_GUEST=1` is set. Commands like `set-window-status` route
/// through RPC instead of calling tmux directly.
pub struct RpcClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl RpcClient {
    /// Connect using WM_RPC_HOST, WM_RPC_PORT, and WM_RPC_TOKEN env vars.
    pub fn from_env() -> Result<Self> {
        let host = std::env::var("WM_RPC_HOST").context("WM_RPC_HOST not set")?;
        let port: u16 = std::env::var("WM_RPC_PORT")
            .context("WM_RPC_PORT not set")?
            .parse()
            .context("WM_RPC_PORT is not a valid port")?;
        let token = std::env::var("WM_RPC_TOKEN").context("WM_RPC_TOKEN not set")?;

        Self::connect(&host, port, &token)
    }

    /// Connect to a specific host, port, and authenticate with token.
    pub fn connect(host: &str, port: u16, token: &str) -> Result<Self> {
        let stream = TcpStream::connect(format!("{}:{}", host, port))
            .with_context(|| format!("Failed to connect to RPC server at {}:{}", host, port))?;

        let writer = stream.try_clone().context("Failed to clone TCP stream")?;
        let reader = BufReader::new(stream);

        // Send auth header
        let auth = AuthHeader {
            token: token.to_string(),
        };
        let mut auth_json = serde_json::to_string(&auth)?;
        auth_json.push('\n');
        let writer_ref = &writer;
        (&*writer_ref).write_all(auth_json.as_bytes())?;
        (&*writer_ref).flush()?;

        Ok(Self { reader, writer })
    }

    /// Send a request and receive a response.
    pub fn call(&mut self, request: &RpcRequest) -> Result<RpcResponse> {
        self.send(request)?;
        self.recv()
    }

    /// Send a request without waiting for a response.
    pub fn send(&mut self, request: &RpcRequest) -> Result<()> {
        let mut req_json = serde_json::to_string(request)?;
        req_json.push('\n');
        (&self.writer).write_all(req_json.as_bytes())?;
        (&self.writer).flush()?;
        Ok(())
    }

    /// Receive a single response line.
    pub fn recv(&mut self) -> Result<RpcResponse> {
        let mut line = String::new();
        self.reader.read_line(&mut line)?;
        serde_json::from_str(&line)
            .with_context(|| format!("Failed to parse RPC response: {}", line))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiplexer;

    #[test]
    fn test_request_serialization_heartbeat() {
        let req = RpcRequest::Heartbeat;
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"Heartbeat\""));
    }

    #[test]
    fn test_request_serialization_set_status() {
        let req = RpcRequest::SetStatus {
            status: "working".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"SetStatus\""));
        assert!(json.contains("\"status\":\"working\""));
    }

    #[test]
    fn test_request_serialization_spawn_agent() {
        let req = RpcRequest::SpawnAgent {
            prompt: "fix the bug".to_string(),
            branch_name: Some("fix-bug".to_string()),
            background: Some(true),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: RpcRequest = serde_json::from_str(&json).unwrap();
        match parsed {
            RpcRequest::SpawnAgent {
                prompt,
                branch_name,
                background,
            } => {
                assert_eq!(prompt, "fix the bug");
                assert_eq!(branch_name.as_deref(), Some("fix-bug"));
                assert_eq!(background, Some(true));
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_response_serialization() {
        let resp = RpcResponse::Ok;
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"Ok\""));

        let resp = RpcResponse::Error {
            message: "oops".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"Error\""));
        assert!(json.contains("\"message\":\"oops\""));
    }

    #[test]
    fn test_request_roundtrip_deserialization() {
        let cases = vec![
            r#"{"type":"Heartbeat"}"#,
            r#"{"type":"SetStatus","status":"working"}"#,
            r#"{"type":"SetTitle","title":"my agent"}"#,
            r#"{"type":"SpawnAgent","prompt":"do stuff","branch_name":null,"background":null}"#,
            r#"{"type":"Exec","command":"cargo","args":["build","--release"]}"#,
            r#"{"type":"Merge","name":"feat","into":null,"rebase":true,"squash":false,"ignore_uncommitted":false,"keep":false,"no_verify":false,"no_hooks":false,"notification":false}"#,
        ];
        for json in cases {
            let req: RpcRequest = serde_json::from_str(json).unwrap();
            // Verify it round-trips
            let re_json = serde_json::to_string(&req).unwrap();
            let _: RpcRequest = serde_json::from_str(&re_json).unwrap();
        }
    }

    #[test]
    fn test_generate_token_is_nonempty() {
        let token = generate_token();
        assert!(!token.is_empty());
        assert_eq!(token.len(), 64, "token should be 64 hex chars (32 bytes)");
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_server_bind_assigns_port() {
        let server = RpcServer::bind().unwrap();
        assert!(server.port() > 0);
    }

    #[test]
    fn test_client_server_heartbeat_roundtrip() {
        let server = RpcServer::bind().unwrap();
        let port = server.port();
        let token = generate_token();

        let mux = multiplexer::create_backend(multiplexer::BackendType::Tmux);
        let ctx = Arc::new(RpcContext {
            pane_id: "%0".to_string(),
            worktree_path: PathBuf::from("/tmp/test"),
            mux,
            token: token.clone(),
            allowed_commands: std::collections::HashSet::new(),
            detected_toolchain: crate::sandbox::toolchain::DetectedToolchain::None,
        });

        let _handle = server.spawn(ctx);

        // Give server thread a moment to start
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut client = RpcClient::connect("127.0.0.1", port, &token).unwrap();
        let resp = client.call(&RpcRequest::Heartbeat).unwrap();
        match resp {
            RpcResponse::Ok => {}
            other => panic!("Expected Ok, got {:?}", other),
        }
    }

    #[test]
    fn test_request_serialization_exec() {
        let req = RpcRequest::Exec {
            command: "just".to_string(),
            args: vec!["check".to_string()],
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"Exec\""));
        assert!(json.contains("\"command\":\"just\""));

        let parsed: RpcRequest = serde_json::from_str(&json).unwrap();
        match parsed {
            RpcRequest::Exec { command, args } => {
                assert_eq!(command, "just");
                assert_eq!(args, vec!["check"]);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_response_serialization_exec_output() {
        let resp = RpcResponse::ExecOutput {
            data: "hello\n".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"ExecOutput\""));
        assert!(json.contains("\"data\":\"hello\\n\""));
    }

    #[test]
    fn test_response_serialization_exec_exit() {
        let resp = RpcResponse::ExecExit { code: 42 };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"ExecExit\""));
        assert!(json.contains("\"code\":42"));

        let parsed: RpcResponse = serde_json::from_str(&json).unwrap();
        match parsed {
            RpcResponse::ExecExit { code } => assert_eq!(code, 42),
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_read_bounded_line_normal() {
        let data = b"hello world\nsecond line\n";
        let mut reader = std::io::BufReader::new(&data[..]);
        let mut buf = String::new();

        let result = read_bounded_line(&mut reader, &mut buf).unwrap();
        assert!(result.is_some());
        assert_eq!(buf, "hello world\n");

        let result = read_bounded_line(&mut reader, &mut buf).unwrap();
        assert!(result.is_some());
        assert_eq!(buf, "second line\n");

        let result = read_bounded_line(&mut reader, &mut buf).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_sanitized_env_normalizes_path() {
        // Test the normalization logic directly without modifying env
        // (set_var is unsafe in Rust 2024 edition due to thread safety)
        let envs = sanitized_env();
        if let Some(path) = envs.get("PATH") {
            for entry in path.split(':') {
                assert!(
                    entry.starts_with('/'),
                    "PATH should only have absolute entries, found: '{}'",
                    entry
                );
            }
        }
    }

    #[test]
    fn test_sanitized_env_excludes_secrets() {
        let envs = sanitized_env();
        // Common secret env vars should never appear
        assert!(!envs.contains_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!envs.contains_key("GITHUB_TOKEN"));
        assert!(!envs.contains_key("WM_RPC_TOKEN"));
        // Only allowlisted keys should be present
        for key in envs.keys() {
            assert!(
                EXEC_ENV_ALLOWLIST.contains(&key.as_str()),
                "unexpected env key in sanitized env: {}",
                key
            );
        }
    }

    #[test]
    fn test_read_bounded_line_rejects_oversized() {
        // Create a line that exceeds MAX_REQUEST_LINE
        let huge = "x".repeat(MAX_REQUEST_LINE + 1);
        let data = format!("{}\n", huge);
        let mut reader = std::io::BufReader::new(data.as_bytes());
        let mut buf = String::new();

        let result = read_bounded_line(&mut reader, &mut buf);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exceeds"));
    }

    #[test]
    fn test_client_server_invalid_token() {
        let server = RpcServer::bind().unwrap();
        let port = server.port();
        let token = generate_token();

        let mux = multiplexer::create_backend(multiplexer::BackendType::Tmux);
        let ctx = Arc::new(RpcContext {
            pane_id: "%0".to_string(),
            worktree_path: PathBuf::from("/tmp/test"),
            mux,
            token: token.clone(),
            allowed_commands: std::collections::HashSet::new(),
            detected_toolchain: crate::sandbox::toolchain::DetectedToolchain::None,
        });

        let _handle = server.spawn(ctx);
        std::thread::sleep(std::time::Duration::from_millis(50));

        let mut client = RpcClient::connect("127.0.0.1", port, "wrong-token").unwrap();
        let resp = client.call(&RpcRequest::Heartbeat).unwrap();
        match resp {
            RpcResponse::Error { message } => assert!(message.contains("Invalid token")),
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    // ── Host-exec integration tests ─────────────────────────────────────

    /// Start an RPC server with the given allowed commands and return a
    /// connected client. Uses a temp dir as the worktree path.
    fn start_exec_server(
        allowed: &[&str],
    ) -> (RpcClient, tempfile::TempDir, thread::JoinHandle<()>) {
        let server = RpcServer::bind().unwrap();
        let port = server.port();
        let token = generate_token();
        let tmp = tempfile::tempdir().unwrap();

        let mux = multiplexer::create_backend(multiplexer::BackendType::Tmux);
        let ctx = Arc::new(RpcContext {
            pane_id: "%0".to_string(),
            worktree_path: tmp.path().to_path_buf(),
            mux,
            token: token.clone(),
            allowed_commands: allowed.iter().map(|s| s.to_string()).collect(),
            detected_toolchain: crate::sandbox::toolchain::DetectedToolchain::None,
        });

        let handle = server.spawn(ctx);
        std::thread::sleep(std::time::Duration::from_millis(50));

        let client = RpcClient::connect("127.0.0.1", port, &token).unwrap();
        (client, tmp, handle)
    }

    /// Send an exec request and collect all streaming responses into
    /// (stdout, stderr, exit_code).
    fn exec_collect(client: &mut RpcClient, command: &str, args: &[&str]) -> (String, String, i32) {
        client
            .send(&RpcRequest::Exec {
                command: command.to_string(),
                args: args.iter().map(|s| s.to_string()).collect(),
            })
            .unwrap();

        let mut stdout = String::new();
        let mut stderr = String::new();
        loop {
            match client.recv().unwrap() {
                RpcResponse::ExecOutput { data } => stdout.push_str(&data),
                RpcResponse::ExecError { data } => stderr.push_str(&data),
                RpcResponse::ExecExit { code } => return (stdout, stderr, code),
                other => panic!("Unexpected response: {:?}", other),
            }
        }
    }

    #[test]
    fn test_exec_allowed_command() {
        let (mut client, _tmp, _handle) = start_exec_server(&["echo"]);
        let (stdout, _stderr, code) = exec_collect(&mut client, "echo", &["hello", "world"]);
        assert_eq!(code, 0);
        assert_eq!(stdout.trim(), "hello world");
    }

    #[test]
    fn test_exec_disallowed_command() {
        let (mut client, _tmp, _handle) = start_exec_server(&["echo"]);
        let (_stdout, _stderr, code) = exec_collect(&mut client, "ls", &[]);
        assert_eq!(code, 127, "disallowed command should return 127");
    }

    #[test]
    fn test_exec_invalid_command_name() {
        let (mut client, _tmp, _handle) = start_exec_server(&["echo"]);

        // Shell metacharacters in command name
        let (_stdout, _stderr, code) = exec_collect(&mut client, "echo;whoami", &[]);
        assert_eq!(code, 127);

        // Path traversal
        let (_stdout, _stderr, code) = exec_collect(&mut client, "/bin/echo", &[]);
        assert_eq!(code, 127);
    }

    #[test]
    fn test_exec_shell_metacharacters_in_args_not_interpreted() {
        let (mut client, _tmp, _handle) = start_exec_server(&["echo"]);

        // $(whoami) should be printed literally, not expanded
        let (stdout, _stderr, code) = exec_collect(&mut client, "echo", &["$(whoami)"]);
        assert_eq!(code, 0);
        assert_eq!(stdout.trim(), "$(whoami)");

        // Backtick substitution should be literal
        let (stdout, _stderr, code) = exec_collect(&mut client, "echo", &["`whoami`"]);
        assert_eq!(code, 0);
        assert_eq!(stdout.trim(), "`whoami`");

        // Semicolons should be literal
        let (stdout, _stderr, code) = exec_collect(&mut client, "echo", &["hello; whoami"]);
        assert_eq!(code, 0);
        assert_eq!(stdout.trim(), "hello; whoami");
    }

    #[test]
    fn test_exec_env_sanitized() {
        let (mut client, _tmp, _handle) = start_exec_server(&["env"]);
        let (stdout, _stderr, code) = exec_collect(&mut client, "env", &[]);
        assert_eq!(code, 0);

        // The RPC token is in our process env but should NOT leak to child
        let env_lines: Vec<&str> = stdout.lines().collect();
        assert!(
            !env_lines.iter().any(|l| l.starts_with("WM_RPC_TOKEN=")),
            "WM_RPC_TOKEN should not be in child environment"
        );

        // PATH should still be present (it's in the allowlist)
        assert!(
            env_lines.iter().any(|l| l.starts_with("PATH=")),
            "PATH should be in child environment"
        );

        // PATH should contain only absolute entries (normalized)
        let path_line = env_lines.iter().find(|l| l.starts_with("PATH=")).unwrap();
        let path_val = &path_line["PATH=".len()..];
        for entry in path_val.split(':') {
            assert!(
                entry.starts_with('/'),
                "PATH entry should be absolute: {}",
                entry
            );
        }
    }

    #[test]
    fn test_exec_sandbox_blocks_ssh_read() {
        // This test verifies the sandbox-exec integration on macOS.
        // On Linux without bwrap, the sandbox falls back to unsandboxed,
        // so we skip the test there.
        #[cfg(target_os = "linux")]
        if which::which("bwrap").is_err() {
            return; // sandbox unavailable by design
        }

        let (mut client, _tmp, _handle) = start_exec_server(&["ls"]);
        let home = std::env::var("HOME").unwrap();
        let ssh_dir = format!("{}/.ssh", home);

        // If ~/.ssh doesn't exist, skip (CI environments)
        if !std::path::Path::new(&ssh_dir).exists() {
            return;
        }

        let (_stdout, stderr, code) = exec_collect(&mut client, "ls", &[&ssh_dir]);
        // sandbox-exec/bwrap should deny access, causing ls to fail
        assert_ne!(
            code,
            0,
            "ls ~/.ssh should fail under sandbox (stderr: {})",
            stderr.trim()
        );
    }

    #[test]
    fn test_exec_nonexistent_command() {
        let (mut client, _tmp, _handle) =
            start_exec_server(&["this-command-definitely-does-not-exist-xyz"]);
        let (_stdout, _stderr, code) = exec_collect(
            &mut client,
            "this-command-definitely-does-not-exist-xyz",
            &[],
        );
        // Should fail to spawn, not hang
        assert_ne!(code, 0);
    }

    #[test]
    fn test_exec_exit_code_propagated() {
        let (mut client, _tmp, _handle) = start_exec_server(&["sh"]);
        let (_stdout, _stderr, code) = exec_collect(&mut client, "sh", &["-c", "exit 42"]);
        assert_eq!(code, 42, "exit code should be propagated from child");
    }

    #[test]
    fn test_exec_stderr_captured() {
        let (mut client, _tmp, _handle) = start_exec_server(&["sh"]);
        let (_stdout, stderr, code) = exec_collect(&mut client, "sh", &["-c", "echo oops >&2"]);
        assert_eq!(code, 0);
        assert_eq!(stderr.trim(), "oops");
    }

    #[test]
    fn test_exec_multiple_commands_on_same_connection() {
        let (mut client, _tmp, _handle) = start_exec_server(&["echo"]);

        let (stdout1, _, code1) = exec_collect(&mut client, "echo", &["first"]);
        assert_eq!(code1, 0);
        assert_eq!(stdout1.trim(), "first");

        let (stdout2, _, code2) = exec_collect(&mut client, "echo", &["second"]);
        assert_eq!(code2, 0);
        assert_eq!(stdout2.trim(), "second");
    }

    #[test]
    fn test_request_serialization_merge() {
        let req = RpcRequest::Merge {
            name: "feature-x".to_string(),
            into: Some("main".to_string()),
            rebase: true,
            squash: false,
            ignore_uncommitted: false,
            keep: true,
            no_verify: false,
            no_hooks: true,
            notification: true,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"Merge\""));
        assert!(json.contains("\"name\":\"feature-x\""));
        assert!(json.contains("\"rebase\":true"));
        assert!(json.contains("\"keep\":true"));
        assert!(json.contains("\"no_hooks\":true"));
        assert!(json.contains("\"notification\":true"));

        // Roundtrip
        let parsed: RpcRequest = serde_json::from_str(&json).unwrap();
        match parsed {
            RpcRequest::Merge {
                name,
                into,
                rebase,
                squash,
                ignore_uncommitted,
                keep,
                no_verify,
                no_hooks,
                notification,
            } => {
                assert_eq!(name, "feature-x");
                assert_eq!(into.as_deref(), Some("main"));
                assert!(rebase);
                assert!(!squash);
                assert!(!ignore_uncommitted);
                assert!(keep);
                assert!(!no_verify);
                assert!(no_hooks);
                assert!(notification);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_response_serialization_output() {
        let resp = RpcResponse::Output {
            message: "Merged 'feature' into 'main'".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"type\":\"Output\""));
        let parsed: RpcResponse = serde_json::from_str(&json).unwrap();
        match parsed {
            RpcResponse::Output { message } => {
                assert_eq!(message, "Merged 'feature' into 'main'");
            }
            _ => panic!("Wrong variant"),
        }
    }
}
