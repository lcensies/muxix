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

/// Turn an image reference into a safe filename component.
///
/// Replaces `/` and `:` with `-`, e.g.
/// `ghcr.io/raine/workmux-sandbox:claude` becomes
/// `ghcr.io-raine-workmux-sandbox-claude`.
fn image_to_filename(image: &str) -> String {
    image.replace(['/', ':'], "-")
}

/// Get the cache directory, optionally rooted at `base` (for testing).
fn cache_dir_in(base: Option<&std::path::Path>) -> Result<PathBuf> {
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

    Ok(state_dir)
}

/// Get the per-image cache file path, optionally rooted at `base` (for testing).
fn cache_file_path_in(base: Option<&std::path::Path>, image: &str) -> Result<PathBuf> {
    let dir = cache_dir_in(base)?;
    Ok(dir.join(format!("image-freshness-{}.json", image_to_filename(image))))
}

/// Get the per-image cache file path.
fn cache_file_path(image: &str) -> Result<PathBuf> {
    cache_file_path_in(None, image)
}

/// Load cached freshness check result.
fn load_cache(image: &str) -> Option<FreshnessCache> {
    let cache_path = cache_file_path(image).ok()?;
    if !cache_path.exists() {
        return None;
    }

    let contents = fs::read_to_string(&cache_path).ok()?;
    let cache: FreshnessCache = serde_json::from_str(&contents).ok()?;

    // Check if cache is still valid (within TTL)
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(cache.checked_at) > CACHE_TTL_SECONDS {
        return None;
    }

    Some(cache)
}

/// Save freshness check result to cache.
fn save_cache(image: &str, is_fresh: bool, local_image_id: Option<String>) -> Result<()> {
    let cache_path = cache_file_path(image)?;
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
///
/// For Docker/Podman, uses `--format "{{.Id}}"`.
/// For Apple Container (which doesn't support `--format`), extracts
/// `index.digest` from the JSON output.
fn get_local_image_id(runtime: SandboxRuntime, image: &str) -> Result<String> {
    if matches!(runtime, SandboxRuntime::AppleContainer) {
        return get_apple_index_digest(image);
    }

    let runtime_bin = runtime.binary_name();
    let output = Command::new(runtime_bin)
        .args(["image", "inspect", "--format", "{{.Id}}", image])
        .output()
        .with_context(|| format!("Failed to run {} image inspect", runtime_bin))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Image inspect failed: {}", stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Extract `index.digest` from `container image inspect` JSON output.
///
/// Apple Container returns a JSON array where each element has an `index`
/// object containing a `digest` field (the OCI image index digest).
fn get_apple_index_digest(image: &str) -> Result<String> {
    let output = Command::new("container")
        .args(["image", "inspect", image])
        .output()
        .context("Failed to run container image inspect")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("container image inspect failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(stdout.trim()).context("Failed to parse container inspect JSON")?;

    json.as_array()
        .and_then(|arr| arr.first())
        .and_then(|entry| entry.pointer("/index/digest"))
        .and_then(|d| d.as_str())
        .map(|s| s.to_string())
        .context("No index.digest in container inspect output")
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

/// Get the current remote manifest digest.
///
/// Uses runtime-appropriate tooling:
/// - Docker: `docker buildx imagetools inspect` (parses `Digest:` line)
/// - Podman: `podman manifest inspect` (parses JSON `digest` field from first manifest)
/// - Apple Container: OCI registry HTTP API via `curl` (ghcr.io token + HEAD request)
fn get_remote_digest(image: &str, runtime: SandboxRuntime) -> Result<String> {
    match runtime {
        SandboxRuntime::Podman => get_remote_digest_podman(image),
        SandboxRuntime::AppleContainer => get_remote_digest_apple(image),
        _ => get_remote_digest_docker(image),
    }
}

fn get_remote_digest_docker(image: &str) -> Result<String> {
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

fn get_remote_digest_podman(image: &str) -> Result<String> {
    let output = Command::new("podman")
        .args(["manifest", "inspect", image])
        .output()
        .context("Failed to run podman manifest inspect")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("podman manifest inspect failed: {}", stderr.trim());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value =
        serde_json::from_str(stdout.trim()).context("Failed to parse manifest JSON")?;

    // OCI image index: look for digest in manifests array
    if let Some(manifests) = json.get("manifests").and_then(|m| m.as_array()) {
        for manifest in manifests {
            if let Some(digest) = manifest.get("digest").and_then(|d| d.as_str())
                && digest.starts_with("sha256:")
            {
                return Ok(digest.to_string());
            }
        }
    }

    anyhow::bail!("Could not find digest in podman manifest output");
}

/// Get remote digest for Apple Container via OCI registry HTTP API.
///
/// Apple Container has no remote inspect command, so we query ghcr.io directly
/// using `curl`:
/// 1. Get an anonymous bearer token from `ghcr.io/token`
/// 2. HEAD the manifest endpoint to read `Docker-Content-Digest` header
fn get_remote_digest_apple(image: &str) -> Result<String> {
    let without_registry = image
        .strip_prefix("ghcr.io/")
        .context("Apple Container freshness check only supports ghcr.io images")?;
    let (repo, tag) = without_registry
        .rsplit_once(':')
        .unwrap_or((without_registry, "latest"));

    // Get anonymous bearer token
    let token_url = format!("https://ghcr.io/token?scope=repository:{}:pull", repo);
    let token_output = Command::new("curl")
        .args(["-sf", &token_url])
        .output()
        .context("Failed to run curl for ghcr.io token")?;

    if !token_output.status.success() {
        anyhow::bail!("Failed to get ghcr.io bearer token");
    }

    let token_json: serde_json::Value =
        serde_json::from_slice(&token_output.stdout).context("Failed to parse token response")?;
    let token = token_json
        .get("token")
        .and_then(|t| t.as_str())
        .context("No token in ghcr.io response")?;

    // HEAD request for manifest digest
    let manifest_url = format!("https://ghcr.io/v2/{}/manifests/{}", repo, tag);
    let head_output = Command::new("curl")
        .args([
            "-sfI",
            "-H",
            &format!("Authorization: Bearer {}", token),
            "-H",
            "Accept: application/vnd.oci.image.index.v1+json",
            "-H",
            "Accept: application/vnd.docker.distribution.manifest.list.v2+json",
            &manifest_url,
        ])
        .output()
        .context("Failed to run curl for manifest HEAD")?;

    if !head_output.status.success() {
        anyhow::bail!("Failed to fetch manifest from ghcr.io");
    }

    // Parse Docker-Content-Digest header (case-insensitive)
    let headers = String::from_utf8_lossy(&head_output.stdout);
    for line in headers.lines() {
        if let Some((key, value)) = line.split_once(':')
            && key.trim().eq_ignore_ascii_case("docker-content-digest")
        {
            let digest = value.trim();
            if digest.starts_with("sha256:") {
                return Ok(digest.to_string());
            }
        }
    }

    anyhow::bail!("No Docker-Content-Digest header in ghcr.io response");
}

/// Perform the freshness check. Returns true if local image matches remote.
///
/// Does NOT print any hints; callers decide how to react.
pub fn check_freshness(image: &str, runtime: SandboxRuntime) -> Result<bool> {
    // Get the current remote manifest digest (e.g. "sha256:abc...")
    let remote_digest =
        get_remote_digest(image, runtime).context("Failed to get remote image digest")?;

    if matches!(runtime, SandboxRuntime::AppleContainer) {
        // Apple Container: compare index.digest directly against remote
        let local_digest =
            get_apple_index_digest(image).context("Failed to get local Apple Container digest")?;
        return Ok(local_digest == remote_digest);
    }

    let runtime_bin = runtime.binary_name();

    // Docker/Podman: compare RepoDigests against remote
    let local_digests =
        get_local_repo_digests(runtime_bin, image).context("Failed to get local image digests")?;

    let is_fresh = local_digests.iter().any(|d| d.contains(&remote_digest));

    Ok(is_fresh)
}

/// Check if an image is from the official workmux registry.
///
/// Matches `ghcr.io/raine/workmux-sandbox:tag` but not
/// `ghcr.io/raine/workmux-sandbox-dev:tag`.
pub fn is_official_image(image: &str) -> bool {
    image
        .strip_prefix(DEFAULT_IMAGE_REGISTRY)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(':') || rest.starts_with('@'))
}

/// Check if the cached freshness status says the image is stale.
///
/// Returns `Some(true)` if cached as stale (and local image hasn't changed),
/// `Some(false)` if cached as fresh, `None` if no valid cache entry.
pub fn cached_is_stale(image: &str, runtime: SandboxRuntime) -> Option<bool> {
    let cache = load_cache(image)?;
    if cache.is_fresh {
        return Some(false);
    }

    // Cached as stale: verify local image hasn't changed since
    if let Ok(current_id) = get_local_image_id(runtime, image)
        && cache.local_image_id.as_deref() == Some(&current_id)
    {
        Some(true)
    } else {
        // Local image changed or couldn't be checked, cache is inconclusive
        None
    }
}

/// Mark an image as fresh in the cache.
///
/// Call this after a successful `sandbox pull` so the staleness hint
/// is not shown until the next TTL window.
pub fn mark_fresh(image: &str, runtime: SandboxRuntime) {
    let local_id = get_local_image_id(runtime, image).ok();
    let _ = save_cache(image, true, local_id);
}

/// Update the freshness cache in background (non-blocking).
///
/// Spawns a detached thread that:
/// 1. Checks if image is from official registry (returns early if not)
/// 2. Checks cache (returns early if recently checked and fresh)
/// 3. Compares local vs remote digests
/// 4. Updates cache with result
///
/// Does not print hints or trigger pulls. The synchronous preflight
/// in `ensure_image_ready` handles those actions using the cached state.
///
/// Silent on any failure (network issues, missing commands, etc.)
pub fn check_in_background(image: String, runtime: SandboxRuntime) {
    std::thread::spawn(move || {
        // Only check official images from our registry
        if !is_official_image(&image) {
            return;
        }

        // Check cache first - if fresh, nothing to do
        if let Some(cache) = load_cache(&image) {
            if cache.is_fresh {
                return;
            }

            // Cached as stale: check if the local image has changed since then
            // (e.g. user ran `docker pull` or auto-pull updated it).
            if let Ok(current_id) = get_local_image_id(runtime, &image)
                && cache.local_image_id.as_deref() == Some(&current_id)
            {
                // Same local image, still stale - no need to re-check
                return;
            }
            // Local image changed or couldn't be checked - fall through to re-check
        }

        // Perform freshness check
        let local_id = get_local_image_id(runtime, &image).ok();
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
        let path =
            cache_file_path_in(Some(tmp.path()), "ghcr.io/raine/workmux-sandbox:claude").unwrap();
        assert!(path.to_string_lossy().contains("workmux"));
        assert!(
            path.to_string_lossy()
                .ends_with("image-freshness-ghcr.io-raine-workmux-sandbox-claude.json")
        );
        // Verify the directory was actually created
        assert!(path.parent().unwrap().is_dir());
    }

    #[test]
    fn test_cache_file_path_per_image() {
        let tmp = tempfile::tempdir().unwrap();
        let path_claude =
            cache_file_path_in(Some(tmp.path()), "ghcr.io/raine/workmux-sandbox:claude").unwrap();
        let path_codex =
            cache_file_path_in(Some(tmp.path()), "ghcr.io/raine/workmux-sandbox:codex").unwrap();
        assert_ne!(path_claude, path_codex);
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

    #[test]
    fn test_parse_apple_container_index_digest() {
        let json = r#"[{"index":{"mediaType":"application/vnd.oci.image.index.v1+json","size":1609,"digest":"sha256:abc123"},"variants":[],"name":"ghcr.io/raine/workmux-sandbox:claude"}]"#;
        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let digest = parsed
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|entry| entry.pointer("/index/digest"))
            .and_then(|d| d.as_str())
            .unwrap();
        assert_eq!(digest, "sha256:abc123");
    }

    #[test]
    fn test_parse_ghcr_docker_content_digest() {
        let headers = "HTTP/2 200\r\ncontent-type: application/vnd.oci.image.index.v1+json\r\nDocker-Content-Digest: sha256:abc123\r\n";
        let mut found = None;
        for line in headers.lines() {
            if let Some((key, value)) = line.split_once(':') {
                if key.trim().eq_ignore_ascii_case("docker-content-digest") {
                    let digest = value.trim();
                    if digest.starts_with("sha256:") {
                        found = Some(digest.to_string());
                    }
                }
            }
        }
        assert_eq!(found.unwrap(), "sha256:abc123");
    }

    #[test]
    fn test_parse_ghcr_docker_content_digest_lowercase() {
        let headers = "HTTP/2 200\r\ndocker-content-digest: sha256:def456\r\n";
        let mut found = None;
        for line in headers.lines() {
            if let Some((key, value)) = line.split_once(':') {
                if key.trim().eq_ignore_ascii_case("docker-content-digest") {
                    let digest = value.trim();
                    if digest.starts_with("sha256:") {
                        found = Some(digest.to_string());
                    }
                }
            }
        }
        assert_eq!(found.unwrap(), "sha256:def456");
    }

    /// Integration tests that require Apple Container and network access.
    /// Run with: cargo test apple_container -- --ignored
    #[test]
    #[ignore]
    fn test_apple_container_local_digest() {
        let digest = get_apple_index_digest("ghcr.io/raine/workmux-sandbox:claude").unwrap();
        assert!(
            digest.starts_with("sha256:"),
            "expected sha256 digest, got: {}",
            digest
        );
    }

    #[test]
    #[ignore]
    fn test_apple_container_remote_digest() {
        let digest = get_remote_digest_apple("ghcr.io/raine/workmux-sandbox:claude").unwrap();
        assert!(
            digest.starts_with("sha256:"),
            "expected sha256 digest, got: {}",
            digest
        );
    }

    #[test]
    #[ignore]
    fn test_apple_container_freshness_check() {
        // A just-pulled image should be fresh
        let is_fresh = check_freshness(
            "ghcr.io/raine/workmux-sandbox:claude",
            SandboxRuntime::AppleContainer,
        )
        .unwrap();
        assert!(is_fresh, "freshly pulled image should be detected as fresh");
    }

    #[test]
    #[ignore]
    fn test_apple_container_digests_match() {
        // Local index.digest and remote Docker-Content-Digest should be identical
        // for a freshly pulled image
        let local = get_apple_index_digest("ghcr.io/raine/workmux-sandbox:claude").unwrap();
        let remote = get_remote_digest_apple("ghcr.io/raine/workmux-sandbox:claude").unwrap();
        assert_eq!(local, remote, "local and remote digests should match");
    }

    #[test]
    fn test_is_official_image() {
        assert!(is_official_image("ghcr.io/raine/workmux-sandbox:claude"));
        assert!(is_official_image("ghcr.io/raine/workmux-sandbox:base"));
        assert!(is_official_image(
            "ghcr.io/raine/workmux-sandbox@sha256:abc"
        ));
        assert!(is_official_image("ghcr.io/raine/workmux-sandbox"));
        assert!(!is_official_image(
            "ghcr.io/raine/workmux-sandbox-dev:claude"
        ));
        assert!(!is_official_image("ghcr.io/raine/workmux-sandboxx:claude"));
        assert!(!is_official_image("docker.io/library/ubuntu:latest"));
    }
}
