use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::GitStatus;

/// Get the path to the git status cache file
pub fn get_cache_path() -> Result<PathBuf> {
    let cache_dir = crate::xdg::cache_dir()?;
    std::fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir.join("git_status_cache.json"))
}

/// Load the git status cache from disk
pub fn load_status_cache() -> HashMap<PathBuf, GitStatus> {
    get_cache_path()
        .map(|p| load_status_cache_at(&p))
        .unwrap_or_default()
}

/// Save the git status cache to disk
pub fn save_status_cache(statuses: &HashMap<PathBuf, GitStatus>) {
    if let Ok(path) = get_cache_path() {
        save_status_cache_at(&path, statuses);
    }
}

/// Load from an explicit path. Split out so a test can prove a saved entry
/// still carries the refresh fingerprint that lets the next poll skip work.
pub(crate) fn load_status_cache_at(path: &Path) -> HashMap<PathBuf, GitStatus> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub(crate) fn save_status_cache_at(path: &Path, statuses: &HashMap<PathBuf, GitStatus>) {
    if let Ok(content) = serde_json::to_string(statuses) {
        let _ = std::fs::write(path, content);
    }
}
