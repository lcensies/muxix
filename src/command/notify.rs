use anyhow::Result;

/// Play a sound notification.
///
/// In a sandbox guest (detected via `WM_SANDBOX_GUEST=1`), routes through RPC
/// to the host supervisor, which runs `afplay` on macOS. This requires the
/// RPC env vars (`WM_RPC_HOST`, `WM_RPC_PORT`, `WM_RPC_TOKEN`) that are set
/// automatically by `workmux sandbox run`.
///
/// Outside a sandbox, runs `afplay` directly (macOS only).
pub fn run_sound(args: &[String]) -> Result<()> {
    if crate::sandbox::guest::is_sandbox_guest() {
        return run_via_rpc(args);
    }

    use std::process::Command;
    let status = Command::new("afplay").args(args).status()?;
    if !status.success() {
        anyhow::bail!("afplay failed with {}", status);
    }
    Ok(())
}

fn run_via_rpc(args: &[String]) -> Result<()> {
    use crate::sandbox::rpc::{NotifyRequest, RpcClient, RpcRequest, RpcResponse};

    let mut client = RpcClient::from_env()?;
    let response = client.call(&RpcRequest::Notify(NotifyRequest::Sound {
        args: args.to_vec(),
    }))?;

    match response {
        RpcResponse::Ok => Ok(()),
        RpcResponse::Error { message } => {
            anyhow::bail!("Notification failed: {}", message);
        }
    }
}
