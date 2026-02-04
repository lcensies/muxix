//! Lima VM backend for sandbox.
//!
//! Provides VM-based sandboxing using Lima (Linux Machines) with configurable isolation levels.

mod config;
mod instance;
mod mounts;
mod wrap;

pub use config::generate_lima_config;
pub use instance::LimaInstance;
pub use mounts::{determine_git_common_dir, determine_project_root, generate_mounts};
pub use wrap::wrap_for_lima;

use crate::config::{Config, IsolationLevel};
use anyhow::Result;
use std::path::Path;

/// Generate a unique instance name for a worktree based on isolation level.
pub fn instance_name(worktree: &Path, isolation: IsolationLevel, _config: &Config) -> Result<String> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let key = match isolation {
        IsolationLevel::User => {
            // Single global VM
            "global".to_string()
        }
        IsolationLevel::Project => {
            // VM per project root (use canonical path for consistency)
            let project_root = determine_project_root(worktree)?;
            let canonical = project_root.canonicalize()
                .unwrap_or_else(|_| project_root.clone());
            canonical.to_string_lossy().to_string()
        }
    };

    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish();

    Ok(format!("wm-{:x}", hash).chars().take(11).collect())
}
