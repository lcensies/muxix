//! TCP RPC protocol for guest-host communication in Lima sandbox.
//!
//! The host-side supervisor runs an RPC server on a random port. The guest
//! workmux binary connects via `host.lima.internal:<port>` and sends
//! JSON-lines requests.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use tracing::{debug, info};

use crate::config::Config;
use crate::multiplexer::Multiplexer;

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
}

/// RPC response sent from host to guest.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RpcResponse {
    Ok,
    Error { message: String },
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
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let pid = std::process::id();
    format!("{:x}{:x}", nanos, pid)
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

        debug!(?request, "RPC request received");
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

    let (icon, auto_clear) = match status.to_lowercase().as_str() {
        "working" => (config.status_icons.working().to_string(), false),
        "waiting" => (config.status_icons.waiting().to_string(), true),
        "done" => (config.status_icons.done().to_string(), true),
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
        Ok(()) => RpcResponse::Ok,
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
        Ok(_) => RpcResponse::Ok,
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
        let mut req_json = serde_json::to_string(request)?;
        req_json.push('\n');
        (&self.writer).write_all(req_json.as_bytes())?;
        (&self.writer).flush()?;

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
