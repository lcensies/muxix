use anyhow::{Result, anyhow};
use std::collections::HashMap;
use std::path::PathBuf;

use super::GitStatus;

/// Get the path to the git status cache file
pub fn get_cache_path() -> Result<PathBuf> {
    let home = home::home_dir().ok_or_else(|| anyhow!("Could not find home directory"))?;
    let cache_dir = home.join(".cache").join("workmux");
    std::fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir.join("git_status_cache.json"))
}

/// Load the git status cache from disk
pub fn load_status_cache() -> HashMap<PathBuf, GitStatus> {
    if let Ok(path) = get_cache_path()
        && path.exists()
        && let Ok(content) = std::fs::read_to_string(&path)
    {
        return serde_json::from_str(&content).unwrap_or_default();
    }
    HashMap::new()
}

/// Save the git status cache to disk
pub fn save_status_cache(statuses: &HashMap<PathBuf, GitStatus>) {
    if let Ok(path) = get_cache_path()
        && let Ok(content) = serde_json::to_string(statuses)
    {
        let _ = std::fs::write(path, content);
    }
}
