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
    /// Local image ID when the check was performed.
    /// Used to invalidate stale cache when the local image changes (e.g. via `docker pull`).
    #[serde(default)]
    local_image_id: Option<String>,
}

/// Get the cache file path, optionally rooted at `base` (for testing).
fn cache_file_path_in(base: Option<&std::path::Path>) -> Result<PathBuf> {
    let state_dir = if let Some(base) = base {
        base.join("workmux")
    } else if let Ok(xdg_state) = std::env::var("XDG_STATE_HOME") {
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

/// Get the cache file path.
fn cache_file_path() -> Result<PathBuf> {
    cache_file_path_in(None)
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
fn save_cache(image: &str, is_fresh: bool, local_image_id: Option<String>) -> Result<()> {
    let cache_path = cache_file_path()?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("Failed to get current time")?
        .as_secs();

    let cache = FreshnessCache {
        image: image.to_string(),
        checked_at: now,
        is_fresh,
        local_image_id,
    };

    let json = serde_json::to_string_pretty(&cache).context("Failed to serialize cache")?;

    fs::write(&cache_path, json)
        .with_context(|| format!("Failed to write cache file: {}", cache_path.display()))?;

    Ok(())
}

/// Get the local image ID (e.g. `sha256:...`).
///
/// This is a cheap local-only operation used to detect when the local image
/// has changed since the last freshness check.
fn get_local_image_id(runtime: &str, image: &str) -> Result<String> {
    let output = Command::new(runtime)
        .args(["image", "inspect", "--format", "{{.Id}}", image])
        .output()
        .with_context(|| format!("Failed to run {} image inspect", runtime))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Image inspect failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Get the repo digests for a local image.
///
/// Returns digests like `["ghcr.io/raine/workmux-sandbox:claude@sha256:abc..."]`.
/// These record the manifest digest the image was originally pulled with.
fn get_local_repo_digests(runtime: &str, image: &str) -> Result<Vec<String>> {
    let output = Command::new(runtime)
        .args([
            "image",
            "inspect",
            "--format",
            "{{json .RepoDigests}}",
            image,
        ])
        .output()
        .with_context(|| format!("Failed to run {} image inspect", runtime))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Image inspect failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let digests: Vec<String> =
        serde_json::from_str(stdout.trim()).context("Failed to parse RepoDigests JSON")?;

    if digests.is_empty() {
        anyhow::bail!("No RepoDigests found (locally built image?)");
    }

    Ok(digests)
}

/// Get the current remote manifest digest via `docker buildx imagetools inspect`.
///
/// Parses the `Digest: sha256:...` line from the text output.
fn get_remote_digest(image: &str) -> Result<String> {
    let output = Command::new("docker")
        .args(["buildx", "imagetools", "inspect", image])
        .output()
        .context("Failed to run docker buildx imagetools inspect")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("imagetools inspect failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(digest) = line.strip_prefix("Digest:") {
            let digest = digest.trim();
            if digest.starts_with("sha256:") {
                return Ok(digest.to_string());
            }
        }
    }

    anyhow::bail!("Could not find Digest in imagetools output");
}

/// Perform the freshness check and print hint if stale.
fn check_freshness(image: &str, runtime: SandboxRuntime) -> Result<bool> {
    let runtime_bin = match runtime {
        SandboxRuntime::Docker => "docker",
        SandboxRuntime::Podman => "podman",
    };

    // Get the digests the local image was pulled with (e.g. "registry/repo@sha256:abc...")
    let local_digests =
        get_local_repo_digests(runtime_bin, image).context("Failed to get local image digests")?;

    // Get the current remote manifest digest (e.g. "sha256:abc...")
    let remote_digest = get_remote_digest(image).context("Failed to get remote image digest")?;

    // Check if any local RepoDigest contains the current remote digest
    let is_fresh = local_digests.iter().any(|d| d.contains(&remote_digest));

    if !is_fresh {
        eprintln!(
            "hint: a newer sandbox image is available (run `workmux sandbox pull` to update)"
        );
    }

    Ok(is_fresh)
}

/// Mark an image as fresh in the cache.
///
/// Call this after a successful `sandbox pull` so the staleness hint
/// is not shown until the next TTL window.
pub fn mark_fresh(image: &str, runtime: SandboxRuntime) {
    let runtime_bin = match runtime {
        SandboxRuntime::Docker => "docker",
        SandboxRuntime::Podman => "podman",
    };
    let local_id = get_local_image_id(runtime_bin, image).ok();
    let _ = save_cache(image, true, local_id);
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

        let runtime_bin = match runtime {
            SandboxRuntime::Docker => "docker",
            SandboxRuntime::Podman => "podman",
        };

        // Check cache first
        if let Some(cache) = load_cache(&image) {
            if cache.is_fresh {
                return;
            }

            // Cached as stale: check if the local image has changed since then
            // (e.g. user ran `docker pull` directly). This is a cheap local check.
            if let Ok(current_id) = get_local_image_id(runtime_bin, &image)
                && cache.local_image_id.as_deref() == Some(&current_id)
            {
                // Same local image, still stale
                eprintln!(
                    "hint: a newer sandbox image is available (run `workmux sandbox pull` to update)"
                );
                return;
            }
            // Local image changed or couldn't be checked - fall through to re-check
        }

        // Perform freshness check
        let local_id = get_local_image_id(runtime_bin, &image).ok();
        match check_freshness(&image, runtime) {
            Ok(is_fresh) => {
                // Save result to cache (ignore errors)
                let _ = save_cache(&image, is_fresh, local_id);
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
        let tmp = tempfile::tempdir().unwrap();
        let path = cache_file_path_in(Some(tmp.path())).unwrap();
        assert!(path.to_string_lossy().contains("workmux"));
        assert!(path.to_string_lossy().ends_with("image-freshness.json"));
        // Verify the directory was actually created
        assert!(path.parent().unwrap().is_dir());
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
            local_image_id: Some("sha256:abc123".to_string()),
        };

        let json = serde_json::to_string(&cache).unwrap();
        let parsed: FreshnessCache = serde_json::from_str(&json).unwrap();

        assert_eq!(cache.image, parsed.image);
        assert_eq!(cache.checked_at, parsed.checked_at);
        assert_eq!(cache.is_fresh, parsed.is_fresh);
        assert_eq!(cache.local_image_id, parsed.local_image_id);
    }

    #[test]
    fn test_freshness_cache_without_local_image_id() {
        // Old cache format without local_image_id should deserialize with None
        let json = r#"{"image":"ghcr.io/raine/workmux-sandbox:claude","checked_at":1707350400,"is_fresh":false}"#;
        let parsed: FreshnessCache = serde_json::from_str(json).unwrap();
        assert!(!parsed.is_fresh);
        assert_eq!(parsed.local_image_id, None);
    }
}
