//! Guest-side detection helpers for sandbox environments.
//!
//! When `WM_SANDBOX_GUEST=1` is set, the workmux binary is running inside
//! a Lima VM and should use RPC instead of direct tmux/host operations.

/// Check if running inside a sandbox guest VM.
pub fn is_sandbox_guest() -> bool {
    std::env::var_os("WM_SANDBOX_GUEST").is_some()
}

/// Get the RPC endpoint from environment variables.
///
/// Returns `(host, port)` if both `WM_RPC_HOST` and `WM_RPC_PORT` are set.
#[allow(dead_code)]
pub fn rpc_endpoint() -> Option<(String, u16)> {
    let host = std::env::var("WM_RPC_HOST").ok()?;
    let port: u16 = std::env::var("WM_RPC_PORT").ok()?.parse().ok()?;
    Some((host, port))
}

/// Get the RPC authentication token from environment.
#[allow(dead_code)]
pub fn rpc_token() -> Option<String> {
    std::env::var("WM_RPC_TOKEN").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_sandbox_guest_when_unset() {
        // Should be false when env var is not set (default in test env)
        // Note: this test assumes WM_SANDBOX_GUEST is not set in the test runner
        if std::env::var_os("WM_SANDBOX_GUEST").is_none() {
            assert!(!is_sandbox_guest());
        }
    }

    #[test]
    fn test_rpc_endpoint_when_unset() {
        // Should be None when env vars are not set
        if std::env::var_os("WM_RPC_HOST").is_none() {
            assert!(rpc_endpoint().is_none());
        }
    }
}
