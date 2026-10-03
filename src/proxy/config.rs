//! Proxy chain configuration for agentgateway + RTK.

use serde::{Deserialize, Serialize};

/// Proxy chain configuration for per-worktree agentgateway setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyChainConfig {
    /// Enable proxy chain for this project
    #[serde(default)]
    pub enabled: bool,

    /// List of proxy hops in order (request flows through them)
    #[serde(default)]
    pub hops: Vec<ProxyHop>,
}

impl Default for ProxyChainConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            hops: vec![
                ProxyHop {
                    name: "gateway".to_string(),
                    hop_type: "gateway".to_string(),
                    endpoint: None,
                    config: None,
                },
                ProxyHop {
                    name: "rtk".to_string(),
                    hop_type: "rtk".to_string(),
                    endpoint: Some("http://localhost:8888".to_string()),
                    config: None,
                },
                ProxyHop {
                    name: "claude-api".to_string(),
                    hop_type: "claude-api".to_string(),
                    endpoint: Some("https://api.anthropic.com".to_string()),
                    config: None,
                },
            ],
        }
    }
}

/// Single hop in the proxy chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyHop {
    /// Name of this hop (e.g., "rtk", "logging")
    pub name: String,

    /// Type of hop: "gateway", "rtk", "claude-api", "custom"
    #[serde(rename = "type")]
    pub hop_type: String,

    /// Optional endpoint for this hop
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,

    /// Optional configuration specific to this hop type
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config: Option<serde_json::Value>,
}

impl ProxyHop {
    /// Create a new RTK hop
    pub fn rtk() -> Self {
        Self {
            name: "rtk".to_string(),
            hop_type: "rtk".to_string(),
            endpoint: Some("http://localhost:8888".to_string()),
            config: None,
        }
    }

    /// Create a new Claude API hop (terminal)
    pub fn claude_api() -> Self {
        Self {
            name: "claude-api".to_string(),
            hop_type: "claude-api".to_string(),
            endpoint: Some("https://api.anthropic.com".to_string()),
            config: None,
        }
    }
}

/// Resolved proxy chain with actual endpoints for a worktree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolvedProxyChain {
    /// Port that agentgateway is listening on
    pub gateway_port: u16,

    /// Full endpoint agents should connect to
    pub agent_api_endpoint: String,

    /// Status of proxy health
    pub healthy: bool,
}

impl ResolvedProxyChain {
    pub fn new(port: u16) -> Self {
        Self {
            gateway_port: port,
            agent_api_endpoint: format!("http://localhost:{}", port),
            healthy: false,
        }
    }
}
