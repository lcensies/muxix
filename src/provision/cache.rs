use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::types::OrgPolicy;

pub enum CacheStatus {
    /// Policy is present and within TTL.
    Fresh(OrgPolicy),
    /// Policy is past TTL but within grace period — still usable, warn.
    Stale(OrgPolicy),
    /// Policy is past grace period — must re-sync.
    Expired,
    /// No policy file on disk.
    Missing,
}

pub fn policy_path() -> Result<PathBuf> {
    Ok(crate::xdg::config_dir()?.join("policy.yaml"))
}

/// Load policy from disk, classifying freshness based on current time.
pub fn load_policy(grace_period_secs: u64) -> Result<CacheStatus> {
    let path = policy_path()?;
    if !path.exists() {
        return Ok(CacheStatus::Missing);
    }

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("reading policy cache at {}", path.display()))?;
    let policy: OrgPolicy = serde_yaml::from_str(&contents)
        .with_context(|| format!("parsing policy cache at {}", path.display()))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if now <= policy.expires_at {
        return Ok(CacheStatus::Fresh(policy));
    }
    if now <= policy.expires_at + grace_period_secs {
        return Ok(CacheStatus::Stale(policy));
    }
    Ok(CacheStatus::Expired)
}

/// Atomically write policy to disk.
pub fn save_policy(policy: &OrgPolicy) -> Result<()> {
    let path = policy_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating config dir {}", parent.display()))?;
    }

    let yaml = serde_yaml::to_string(policy).context("serializing policy")?;
    let tmp = path.with_extension("yaml.tmp");
    std::fs::write(&tmp, &yaml)
        .with_context(|| format!("writing tmp policy at {}", tmp.display()))?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("renaming policy to {}", path.display()))?;
    Ok(())
}
