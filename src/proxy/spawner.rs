//! Spawner for per-worktree agentgateway + RTK proxy chain.
//! Spawning is not implemented yet, so nothing calls into this module.
#![allow(dead_code)]

use anyhow::{Context, Result, anyhow, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::thread;
use std::time::Duration;
use tracing::{debug, info};

use super::config::{ProxyChainConfig, ResolvedProxyChain};

/// Spawns and manages per-worktree agentgateway + RTK proxy chain.
pub struct ProxySpawner {
    worktree_path: PathBuf,
    worktree_handle: String,
    config: ProxyChainConfig,
    gateway_port: u16,
}

impl ProxySpawner {
    pub fn new(
        worktree_path: PathBuf,
        worktree_handle: String,
        config: ProxyChainConfig,
        gateway_port: u16,
    ) -> Self {
        Self {
            worktree_path,
            worktree_handle,
            config,
            gateway_port,
        }
    }

    /// Allocate a unique port for this worktree (deterministic based on handle hash)
    pub fn allocate_port(handle: &str, base_port: u16) -> u16 {
        let hash = handle
            .chars()
            .fold(0u32, |acc, c| acc.wrapping_mul(31).wrapping_add(c as u32));
        base_port + (hash % 1000) as u16
    }

    /// Spawn the agentgateway + RTK proxy chain for this worktree.
    ///
    /// NOTE: the proxy chain is **not yet implemented**. This is gated off by
    /// default (`proxy_chain.enabled = false`) and, even when enabled, returns
    /// an explicit error rather than fabricating a healthy chain. Previously
    /// this spawned `/bin/true` and reported `healthy: true`, so any agent
    /// routed through `agent_api_endpoint` hit connection-refused at request
    /// time. Tracked as a follow-up task (`finish-proxy-chain`).
    pub fn spawn(&self) -> Result<ResolvedProxyChain> {
        if !self.config.enabled {
            bail!(
                "proxy chain is disabled; opt in with `proxy_chain.enabled = true` once it is implemented"
            );
        }

        info!(
            "Spawning proxy chain for worktree '{}' on port {}",
            self.worktree_handle, self.gateway_port
        );

        // 1. Ensure .muxix directory exists
        let muxix_dir = self.worktree_path.join(".muxix");
        fs::create_dir_all(&muxix_dir).context("Failed to create .muxix directory")?;

        // 2. Generate agentgateway config
        let ag_config_path = self.generate_agentgateway_config(&muxix_dir)?;

        // 3. Spawn agentgateway subprocess — currently returns a not-implemented
        //    error so we never persist a non-functional proxy as healthy.
        let _ag_child = self.spawn_agentgateway(&ag_config_path)?;

        // 4. Health check
        self.health_check(10)?;

        // 5. Store resolved config
        let resolved = ResolvedProxyChain {
            gateway_port: self.gateway_port,
            agent_api_endpoint: format!("http://localhost:{}", self.gateway_port),
            healthy: true,
        };

        self.store_resolved_config(&resolved)?;

        info!("Proxy chain ready on {}", resolved.agent_api_endpoint);

        Ok(resolved)
    }

    /// Generate agentgateway config file with routes.
    fn generate_agentgateway_config(&self, muxix_dir: &Path) -> Result<PathBuf> {
        let config = self.build_agentgateway_config()?;
        let config_path = muxix_dir.join("agentgateway.yaml");

        fs::write(&config_path, &config).context("Failed to write agentgateway config")?;

        debug!("Generated agentgateway config at {}", config_path.display());

        Ok(config_path)
    }

    /// Build agentgateway YAML configuration.
    fn build_agentgateway_config(&self) -> Result<String> {
        let mut backends = String::new();

        // Build backends section from hops (skip gateway, include others)
        for hop in &self.config.hops {
            if hop.hop_type == "gateway" {
                continue; // Gateway is the listener, not a backend
            }
            if let Some(endpoint) = &hop.endpoint {
                backends.push_str(&format!(
                    "  {name}:\n    endpoint: {endpoint}\n",
                    name = hop.name,
                    endpoint = endpoint
                ));
            }
        }

        // Build route (in order: rtk → claude-api)
        let mut route_backends = String::new();
        for hop in &self.config.hops {
            if hop.hop_type != "gateway" && hop.endpoint.is_some() {
                route_backends.push_str(&format!("      - {}\n", hop.name));
            }
        }

        let config = format!(
            r#"binds:
  - name: http
    address: 127.0.0.1:{port}
    protocol: HTTP

routes:
  - name: llm-chain
    path: /v1/chat/completions
    backends:
{route_backends}
backends:
{backends}"#,
            port = self.gateway_port,
            route_backends = route_backends,
            backends = backends
        );

        Ok(config)
    }

    /// Spawn agentgateway subprocess.
    fn spawn_agentgateway(&self, config_path: &Path) -> Result<Child> {
        // NOT YET IMPLEMENTED. The real implementation would spawn
        // `agentgateway --config <path>` and return the child handle. Until
        // then we must NOT pretend success (the old code spawned `/bin/true`,
        // which made `spawn()` report a healthy proxy that no agent could reach).
        debug!(
            "agentgateway spawn requested with config {} (not implemented)",
            config_path.display()
        );
        bail!(
            "agentgateway spawning is not yet implemented; refusing to report a non-functional proxy chain as healthy (tracked: finish-proxy-chain)"
        )
    }

    /// Health check: verify proxy is responding
    fn health_check(&self, max_retries: u32) -> Result<()> {
        for attempt in 1..=max_retries {
            debug!(
                "Health check attempt {}/{} on port {}",
                attempt, max_retries, self.gateway_port
            );

            // Give the gateway a moment to start before probing.
            if attempt == 1 {
                thread::sleep(Duration::from_millis(100));
            }

            // TODO: actually probe whether agentgateway is listening on
            // `self.gateway_port` (e.g. TcpStream::connect with a short timeout).
            if std::net::TcpStream::connect(("127.0.0.1", self.gateway_port)).is_ok() {
                return Ok(());
            }
        }

        Err(anyhow!(
            "Proxy chain failed health check after {} attempts",
            max_retries
        ))
    }

    /// Store resolved proxy config in .muxix/config.json
    fn store_resolved_config(&self, resolved: &ResolvedProxyChain) -> Result<()> {
        let config_path = self.worktree_path.join(".muxix/config.json");
        let json = serde_json::to_string_pretty(resolved)
            .context("Failed to serialize resolved config")?;

        fs::write(&config_path, json).context("Failed to write resolved config")?;

        debug!("Stored resolved proxy config at {}", config_path.display());

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_port_deterministic() {
        let port1 = ProxySpawner::allocate_port("feature-x", 7777);
        let port2 = ProxySpawner::allocate_port("feature-x", 7777);
        assert_eq!(port1, port2, "Port allocation should be deterministic");
    }

    #[test]
    fn test_allocate_port_different_handles() {
        let port1 = ProxySpawner::allocate_port("feature-x", 7777);
        let port2 = ProxySpawner::allocate_port("feature-y", 7777);
        assert_ne!(port1, port2, "Different handles should get different ports");
        assert!((7777..8777).contains(&port1), "Port should be in range");
        assert!((7777..8777).contains(&port2), "Port should be in range");
    }

    #[test]
    fn test_build_agentgateway_config() {
        let config = ProxyChainConfig::default();
        let spawner = ProxySpawner::new(
            PathBuf::from("/tmp/test"),
            "test-handle".to_string(),
            config,
            7777,
        );

        let ag_config = spawner.build_agentgateway_config().unwrap();

        assert!(
            ag_config.contains("127.0.0.1:7777"),
            "Should bind to correct port"
        );
        assert!(ag_config.contains("rtk:"), "Should include RTK backend");
        assert!(
            ag_config.contains("claude-api:"),
            "Should include Claude API backend"
        );
        assert!(
            ag_config.contains("/v1/chat/completions"),
            "Should have LLM route"
        );
    }

    #[test]
    fn test_spawn_disabled_by_default_returns_error() {
        // Default config has `enabled = false`; spawn must refuse without any
        // filesystem side effects.
        let config = ProxyChainConfig::default();
        assert!(!config.enabled);
        let spawner = ProxySpawner::new(
            PathBuf::from("/nonexistent/should-not-be-created"),
            "test-handle".to_string(),
            config,
            7777,
        );
        let err = spawner.spawn().unwrap_err().to_string();
        assert!(err.contains("disabled"), "got: {err}");
    }

    #[test]
    fn test_spawn_enabled_does_not_fabricate_healthy_chain() {
        // Even when enabled, spawning is not implemented yet: it must return an
        // error and must NOT write a resolved config claiming `healthy: true`.
        let dir = tempfile::tempdir().unwrap();
        let config = ProxyChainConfig {
            enabled: true,
            ..Default::default()
        };
        let spawner = ProxySpawner::new(
            dir.path().to_path_buf(),
            "test-handle".to_string(),
            config,
            7777,
        );
        let err = spawner.spawn().unwrap_err().to_string();
        assert!(err.contains("not yet implemented"), "got: {err}");
        // No resolved config should have been persisted.
        assert!(!dir.path().join(".muxix/config.json").exists());
    }
}
