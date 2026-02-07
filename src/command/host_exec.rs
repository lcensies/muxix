//! Execute a command on the host via RPC.
//!
//! Used by guest shims to transparently proxy toolchain commands
//! (just, cargo, npm, etc.) to the host machine.

use anyhow::{Result, bail};
use std::io::Write;

use crate::sandbox::rpc::{RpcClient, RpcRequest, RpcResponse};

/// Run a command on the host and stream output to local stdout/stderr.
/// Returns the remote process exit code.
pub fn run(command: &str, args: &[String]) -> Result<i32> {
    if !crate::sandbox::guest::is_sandbox_guest() {
        bail!("host-exec only works inside a sandbox guest (WM_SANDBOX_GUEST=1)");
    }

    let mut client = RpcClient::from_env()?;

    // Send exec request
    let request = RpcRequest::Exec {
        command: command.to_string(),
        args: args.to_vec(),
    };
    client.send(&request)?;

    // Stream responses until ExecExit
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();

    loop {
        let response = client.recv()?;
        match response {
            RpcResponse::ExecOutput { data } => {
                stdout.write_all(data.as_bytes())?;
                stdout.flush()?;
            }
            RpcResponse::ExecError { data } => {
                stderr.write_all(data.as_bytes())?;
                stderr.flush()?;
            }
            RpcResponse::ExecExit { code } => {
                return Ok(code);
            }
            RpcResponse::Error { message } => {
                bail!("Host exec failed: {}", message);
            }
            _ => {
                // Ignore unexpected responses
            }
        }
    }
}
