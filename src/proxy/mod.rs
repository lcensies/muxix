//! Per-worktree proxy chain management (agentgateway + RTK integration).

pub mod config;
pub mod spawner;

pub use config::{ProxyChainConfig, ProxyHop};
pub use spawner::ProxySpawner;
