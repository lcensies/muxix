//! Background image freshness check system.
//!
//! Checks if a newer sandbox image is available by comparing local vs remote digests.
//! Only triggers for official ghcr.io/raine/workmux-sandbox images.
//! Runs in background thread and never blocks startup.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::SandboxRuntime;
use crate::sandbox::DEFAULT_IMAGE_REGISTRY;

/// How long to cache freshness check results (24 hours in seconds).
const CACHE_TTL_SECONDS: u64 = 24 * 60 * 60;

/// Cached freshness check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct FreshnessCache {
    /// Image name that was checked.
    image: String,
    /// Unix timestamp when check was performed.
    checked_at: u64,
    /// Whether the image is fresh (local matches remote).
    is_fresh: bool,
}

/// Get the cache file path.
fn cache_file_path() -> Result<PathBuf> {
    let state_dir = if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
        PathBuf::from(xdg_state).join("workmux")
    } else if let Some(home) = home::home_dir() {
        home.join(".local/state/workmux")
    } else {
        anyhow::bail!("Could not determine state directory");
    };

    fs::create_dir_all(&state_dir)
        .with_context(|| format!("Failed to create state directory: {}", state_dir.display()))?;

    Ok(state_dir.join("image-freshness.json"))
}

/// Load cached freshness check result.
fn load_cache(image: &str) -> Option<FreshnessCache> {
    let cache_path = cache_file_path().ok()?;
    if !cache_path.exists() {
        return None;
    }

    let contents = fs::read_to_string(&cache_path).ok()?;
    let cache: FreshnessCache = serde_json::from_str(&contents).ok()?;

    // Check if cache is for the same image
    if cache.image != image {
        return None;
    }

    // Check if cache is still valid (within TTL)
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(cache.checked_at) > CACHE_TTL_SECONDS {
        return None;
    }

    Some(cache)
}

/// Save freshness check result to cache.
fn save_cache(image: &str, is_fresh: bool) -> Result<()> {
    let cache_path = cache_file_path()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Failed to get current time")?
        .as_secs();

    let cache = FreshnessCache {
        image: image.to_string(),
        checked_at: now,
        is_fresh,
    };

    let json = serde_json::to_string_pretty(&cache).context("Failed to serialize cache")?;

    fs::write(&cache_path, json)
        .with_context(|| format!("Failed to write cache file: {}", cache_path.display()))?;

    Ok(())
}

/// Get local image digest.
fn get_local_digest(runtime: &str, image: &str) -> Result<String> {
    let output = Command::new(runtime)
        .args(["image", "inspect", "--format", "{{.Id}}", image])
        .output()
        .with_context(|| format!("Failed to run {} image inspect", runtime))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Image inspect failed: {}", stderr.trim());
    }

    let digest = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if digest.is_empty() {
        anyhow::bail!("Empty digest from image inspect");
    }

    Ok(digest)
}

/// Get remote image digest by parsing manifest JSON.
fn get_remote_digest(runtime: &str, image: &str) -> Result<String> {
    let output = Command::new(runtime)
        .args(["manifest", "inspect", image])
        .output()
        .with_context(|| format!("Failed to run {} manifest inspect", runtime))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Manifest inspect failed: {}", stderr.trim());
    }

    let json_str = String::from_utf8_lossy(&output.stdout);
    let manifest: serde_json::Value =
        serde_json::from_str(&json_str).context("Failed to parse manifest JSON")?;

    // Try to extract digest from manifest
    // For OCI manifests, the digest might be in different places:
    // - config.digest for single-platform images
    // - manifests[].digest for multi-platform images (we want the overall digest)

    // First, try to get the overall manifest digest from the config
    if let Some(config) = manifest.get("config")
        && let Some(digest) = config.get("digest").and_then(|d| d.as_str())
    {
        return Ok(digest.to_string());
    }

    // Fallback: look for "digest" field at the top level
    if let Some(digest) = manifest.get("digest").and_then(|d| d.as_str()) {
        return Ok(digest.to_string());
    }

    // For multi-platform manifests, we can't easily get a single digest that matches
    // the local image inspect output, so we'll consider this a format we can't handle
    anyhow::bail!("Could not extract digest from manifest");
}

/// Perform the freshness check and print hint if stale.
fn check_freshness(image: &str, runtime: SandboxRuntime) -> Result<bool> {
    let runtime_bin = match runtime {
        SandboxRuntime::Docker => "docker",
        SandboxRuntime::Podman => "podman",
    };

    // Get local digest
    let local_digest =
        get_local_digest(runtime_bin, image).context("Failed to get local image digest")?;

    // Get remote digest
    let remote_digest =
        get_remote_digest(runtime_bin, image).context("Failed to get remote image digest")?;

    // Compare digests
    let is_fresh = local_digest == remote_digest;

    if !is_fresh {
        // Print hint to stderr (non-blocking, doesn't interfere with stdout)
        eprintln!(
            "hint: a newer sandbox image is available (run `workmux sandbox pull` to update)"
        );
    }

    Ok(is_fresh)
}

/// Check image freshness in background (non-blocking).
///
/// Spawns a detached thread that:
/// 1. Checks if image is from official registry (returns early if not)
/// 2. Checks cache (returns early if recently checked)
/// 3. Compares local vs remote digests
/// 4. Prints hint to stderr if stale
/// 5. Updates cache with result
///
/// Silent on any failure (network issues, missing commands, etc.)
pub fn check_in_background(image: String, runtime: SandboxRuntime) {
    std::thread::spawn(move || {
        // Only check official images from our registry
        if !image.starts_with(DEFAULT_IMAGE_REGISTRY) {
            return;
        }

        // Check cache first - if recently checked, skip
        if let Some(cache) = load_cache(&image) {
            // If cached result shows staleness, print hint again
            if !cache.is_fresh {
                eprintln!(
                    "hint: a newer sandbox image is available (run `workmux sandbox pull` to update)"
                );
            }
            return;
        }

        // Perform freshness check
        match check_freshness(&image, runtime) {
            Ok(is_fresh) => {
                // Save result to cache (ignore errors)
                let _ = save_cache(&image, is_fresh);
            }
            Err(_e) => {
                // Silent on failure - don't bother users with network/command issues
                // Uncomment for debugging:
                // eprintln!("debug: freshness check failed: {}", _e);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_file_path() {
        let path = cache_file_path().unwrap();
        assert!(path.to_string_lossy().contains("workmux"));
        assert!(path.to_string_lossy().ends_with("image-freshness.json"));
    }

    #[test]
    fn test_load_cache_missing_file() {
        let result = load_cache("test-image:latest");
        assert!(result.is_none());
    }

    #[test]
    fn test_freshness_cache_serialization() {
        let cache = FreshnessCache {
            image: "ghcr.io/raine/workmux-sandbox:claude".to_string(),
            checked_at: 1707350400,
            is_fresh: true,
        };

        let json = serde_json::to_string(&cache).unwrap();
        let parsed: FreshnessCache = serde_json::from_str(&json).unwrap();

        assert_eq!(cache.image, parsed.image);
        assert_eq!(cache.checked_at, parsed.checked_at);
        assert_eq!(cache.is_fresh, parsed.is_fresh);
    }
}
