use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A sanitised snapshot of a muxix config suitable for team sharing.
/// Never includes secrets (API keys, tokens, env values).
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ProfileSnapshot {
    pub schema_version: u32,
    pub muxix_version: String,
    pub platform: String,
    /// Agent kind stem (e.g. "claude", "gemini") — command path stripped.
    pub agent_kind: Option<String>,
    /// Names of MCP servers (not their commands or env).
    pub mcp_names: Vec<String>,
    /// Provider names from agent definitions.
    pub provider_names: Vec<String>,
    /// Feature flags as key→bool.
    pub features: BTreeMap<String, bool>,
    /// Hashed hostname for correlation without leaking machine identity.
    pub hostname_hash: Option<String>,
}

/// Generate a profile snapshot from the given config.
pub fn generate_snapshot(config: &crate::config::Config) -> ProfileSnapshot {
    let agent_kind = config.agent.as_deref().map(|cmd| {
        cmd.split_whitespace()
            .next()
            .and_then(|s| s.rsplit('/').next())
            .unwrap_or(cmd)
            .to_string()
    });

    let mcp_names = config
        .mcp
        .as_ref()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    // Extract provider names from agent definitions (model field prefix).
    let provider_names: Vec<String> = config
        .agent_defs
        .values()
        .filter_map(|def| {
            def.model
                .as_deref()
                .and_then(|m| m.split('/').next().map(|p| p.to_string()))
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();

    let mut features = BTreeMap::new();
    features.insert(
        "sandbox_enabled".to_string(),
        config.sandbox.enabled.unwrap_or(false),
    );
    features.insert(
        "proxy_chain_enabled".to_string(),
        config
            .proxy_chain
            .as_ref()
            .map(|p| p.enabled)
            .unwrap_or(false),
    );
    features.insert("bootstrap_enabled".to_string(), config.bootstrap.is_some());

    let hostname_hash = hostname_hash();

    ProfileSnapshot {
        schema_version: 1,
        muxix_version: env!("CARGO_PKG_VERSION").to_string(),
        platform: std::env::consts::OS.to_string(),
        agent_kind,
        mcp_names,
        provider_names,
        features,
        hostname_hash,
    }
}

fn hostname_hash() -> Option<String> {
    // Try /etc/hostname first; fall back to HOSTNAME env var.
    let name = std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .or_else(|| std::env::var("HOSTNAME").ok())?;
    if name.is_empty() {
        return None;
    }
    // Simple djb2 hash — sufficient for dedup, not for security.
    let mut h: u64 = 5381;
    for b in name.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    Some(format!("{:016x}", h))
}
