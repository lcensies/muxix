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
use std::thread;
use tracing::{debug, info};

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
    Notify(NotifyRequest),
    Exec {
        command: String,
        args: Vec<String>,
    },
}

/// Typed notification request sent from guest to host.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum NotifyRequest {
    /// Play a sound file on the host.
    Sound { args: Vec<String> },
}

/// RPC response sent from host to guest.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RpcResponse {
    Ok,
    Error { message: String },
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
        thread::spawn(move || {
            for stream in self.listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let ctx = Arc::clone(&ctx);
                        thread::spawn(move || {
                            if let Err(e) = handle_connection(stream, &ctx) {
                                debug!(error = %e, "RPC connection ended");
                            }
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

// ── Connection handler ──────────────────────────────────────────────────

/// Header line sent by client before requests. Contains the auth token.
#[derive(Debug, Serialize, Deserialize)]
struct AuthHeader {
    token: String,
}

fn handle_connection(stream: TcpStream, ctx: &RpcContext) -> Result<()> {
    let peer = stream.peer_addr().ok();
    debug!(?peer, "RPC connection accepted");

    let mut reader = BufReader::new(&stream);
    let mut writer = stream.try_clone().context("Failed to clone TCP stream")?;

    // First line must be auth header
    let mut auth_line = String::new();
    reader.read_line(&mut auth_line)?;
    let auth: AuthHeader =
        serde_json::from_str(auth_line.trim()).context("Failed to parse auth header")?;

    if auth.token != ctx.token {
        let resp = RpcResponse::Error {
            message: "Invalid token".to_string(),
        };
        write_response(&mut writer, &resp)?;
        return Ok(());
    }

    // Process request lines
    for line in reader.lines() {
        let line = line.context("Failed to read RPC request line")?;
        if line.trim().is_empty() {
            continue;
        }

        let request: RpcRequest = serde_json::from_str(&line)
            .with_context(|| format!("Failed to parse RPC request: {}", line))?;

        info!(?request, "RPC request received");

        // Exec requires streaming multiple responses, handle separately
        if let RpcRequest::Exec {
            ref command,
            ref args,
        } = request
        {
            handle_exec(command, args, ctx, &mut writer)?;
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
        RpcRequest::Notify(req) => handle_notify(req),
        RpcRequest::Exec { .. } => {
            // Handled in handle_connection before dispatch
            unreachable!("Exec is handled directly in handle_connection")
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

fn handle_notify(req: &NotifyRequest) -> RpcResponse {
    match req {
        NotifyRequest::Sound { args } => {
            use std::process::Command;
            // Spawn afplay in a detached thread to avoid blocking the RPC response
            // while still reaping the child process (preventing zombies).
            let args = args.clone();
            thread::spawn(move || match Command::new("afplay").args(&args).status() {
                Ok(status) if !status.success() => {
                    debug!(?args, ?status, "afplay exited with error");
                }
                Err(e) => {
                    debug!(?args, error = %e, "failed to run afplay");
                }
                _ => {}
            });
            RpcResponse::Ok
        }
    }
}

fn handle_exec(
    command: &str,
    args: &[String],
    ctx: &RpcContext,
    writer: &mut impl Write,
) -> Result<()> {
    use std::process::{Command, Stdio};

    info!(command, ?args, "host-exec request");

    // Validate command is in allowlist
    if !ctx.allowed_commands.contains(command) {
        let resp = RpcResponse::ExecExit { code: 127 };
        write_response(writer, &resp)?;
        return Ok(());
    }

    // Reject commands containing path separators
    if command.contains('/') || command.contains('\\') {
        let resp = RpcResponse::ExecExit { code: 127 };
        write_response(writer, &resp)?;
        return Ok(());
    }

    let mut child = if ctx.detected_toolchain != crate::sandbox::toolchain::DetectedToolchain::None
    {
        // Wrap with toolchain: build a shell command that enters the env first
        let full_cmd = std::iter::once(command.to_string())
            .chain(args.iter().map(|a| shell_quote(a)))
            .collect::<Vec<_>>()
            .join(" ");
        let wrapped = crate::sandbox::toolchain::wrap_command(&full_cmd, &ctx.detected_toolchain);
        let mut cmd = Command::new("bash");
        cmd.args(["-c", &wrapped]);
        cmd.current_dir(&ctx.worktree_path);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.spawn()
            .with_context(|| format!("Failed to spawn wrapped command: {}", command))?
    } else {
        let mut cmd = Command::new(command);
        cmd.args(args);
        cmd.current_dir(&ctx.worktree_path);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd.spawn()
            .with_context(|| format!("Failed to spawn command: {}", command))?
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

fn shell_quote(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '/')
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
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
    fn test_request_serialization_notify_sound() {
        let req = RpcRequest::Notify(NotifyRequest::Sound {
            args: vec!["/System/Library/Sounds/Glass.aiff".to_string()],
        });
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"type\":\"Notify\""));
        assert!(json.contains("\"kind\":\"Sound\""));
        assert!(json.contains("/System/Library/Sounds/Glass.aiff"));

        // Roundtrip
        let parsed: RpcRequest = serde_json::from_str(&json).unwrap();
        match parsed {
            RpcRequest::Notify(NotifyRequest::Sound { args }) => {
                assert_eq!(args, vec!["/System/Library/Sounds/Glass.aiff"]);
            }
            _ => panic!("Wrong variant"),
        }
    }

    #[test]
    fn test_request_roundtrip_deserialization() {
        let cases = vec![
            r#"{"type":"Heartbeat"}"#,
            r#"{"type":"SetStatus","status":"working"}"#,
            r#"{"type":"SetTitle","title":"my agent"}"#,
            r#"{"type":"SpawnAgent","prompt":"do stuff","branch_name":null,"background":null}"#,
            r#"{"type":"Notify","kind":"Sound","args":["/tmp/beep.aiff"]}"#,
            r#"{"type":"Exec","command":"cargo","args":["build","--release"]}"#,
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
}
