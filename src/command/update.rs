use anyhow::{Context, Result, bail};
use std::process::Command;

const REPO: &str = "raine/workmux";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Map OS/arch to the release artifact suffix used in GitHub releases.
fn platform_suffix() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("macos", "x86_64") => Ok("darwin-amd64"),
        ("linux", "x86_64") => Ok("linux-amd64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        (os, arch) => bail!("Unsupported platform: {os}/{arch}"),
    }
}

/// Check if the binary is managed by Homebrew.
fn is_homebrew_install(exe_path: &std::path::Path) -> bool {
    let path_str = exe_path.to_string_lossy();
    path_str.contains("/Cellar/") || path_str.contains("/homebrew/")
}

/// Fetch the latest release tag from GitHub API using curl.
fn fetch_latest_version() -> Result<String> {
    let output = Command::new("curl")
        .args([
            "-sSf",
            &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        ])
        .output()
        .context("Failed to run curl. Is curl installed?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to fetch latest release: {}", stderr.trim());
    }

    let body: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("Failed to parse GitHub API response")?;

    let tag = body["tag_name"]
        .as_str()
        .context("No tag_name in GitHub API response")?;

    Ok(tag.strip_prefix('v').unwrap_or(tag).to_string())
}

/// Download a URL to a file path using curl.
fn download(url: &str, dest: &std::path::Path) -> Result<()> {
    let status = Command::new("curl")
        .args(["-sSLf", "-o"])
        .arg(dest)
        .arg(url)
        .status()
        .context("Failed to run curl")?;

    if !status.success() {
        bail!("Download failed: {url}");
    }
    Ok(())
}

/// Extract a tar.gz archive into a directory.
fn extract_tar(archive: &std::path::Path, dest: &std::path::Path) -> Result<()> {
    let status = Command::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(dest)
        .status()
        .context("Failed to run tar")?;

    if !status.success() {
        bail!("Failed to extract archive");
    }
    Ok(())
}

/// Compute SHA-256 hash of a file using system tools.
fn sha256_of(path: &std::path::Path) -> Result<String> {
    // Try sha256sum first (common on Linux)
    if let Ok(output) = Command::new("sha256sum").arg(path).output()
        && output.status.success()
    {
        let out = String::from_utf8_lossy(&output.stdout);
        if let Some(hash) = out.split_whitespace().next() {
            return Ok(hash.to_string());
        }
    }

    // Fall back to shasum -a 256 (macOS)
    let output = Command::new("shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()
        .context("Neither sha256sum nor shasum found. Cannot verify checksum.")?;

    if !output.status.success() {
        bail!("Checksum command failed");
    }

    let out = String::from_utf8_lossy(&output.stdout);
    out.split_whitespace()
        .next()
        .map(|s| s.to_string())
        .context("Could not parse checksum output")
}

/// Verify SHA-256 checksum of a file against the expected checksum line.
fn verify_checksum(file: &std::path::Path, expected_line: &str) -> Result<()> {
    // The .sha256 file format is: "<hash>  <filename>"
    let expected_hash = expected_line
        .split_whitespace()
        .next()
        .context("Invalid checksum file format")?;

    let actual_hash = sha256_of(file)?;
    if actual_hash != expected_hash {
        bail!("Checksum mismatch!\n  Expected: {expected_hash}\n  Got:      {actual_hash}");
    }
    Ok(())
}

/// Replace the current binary with the new one, with rollback on failure.
fn replace_binary(new_binary: &std::path::Path, current_exe: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let exe_dir = current_exe
        .parent()
        .context("Could not determine binary directory")?;

    // Copy to destination directory to avoid EXDEV (cross-device rename)
    let staged = exe_dir.join(".workmux.new");
    std::fs::copy(new_binary, &staged).context("Failed to copy new binary to install directory")?;
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755))?;

    // Rename current -> .old, then staged -> current
    let backup = exe_dir.join(".workmux.old");
    std::fs::rename(current_exe, &backup).context("Failed to move current binary aside")?;

    if let Err(e) = std::fs::rename(&staged, current_exe) {
        // Rollback: restore the original
        let _ = std::fs::rename(&backup, current_exe);
        return Err(e).context("Failed to install new binary (rolled back)");
    }

    // Cleanup
    let _ = std::fs::remove_file(&backup);
    Ok(())
}

fn do_update(
    pb: &indicatif::ProgressBar,
    artifact_name: &str,
    current_exe: &std::path::Path,
) -> Result<String> {
    let latest_version = fetch_latest_version()?;

    if latest_version == CURRENT_VERSION {
        return Ok(format!("Already up to date (v{CURRENT_VERSION})"));
    }

    pb.set_message(format!("Downloading v{latest_version}..."));

    let tmp = tempfile::tempdir().context("Failed to create temp directory")?;
    let tar_path = tmp.path().join(format!("{artifact_name}.tar.gz"));
    let sha_path = tmp.path().join(format!("{artifact_name}.sha256"));

    let base_url = format!("https://github.com/{REPO}/releases/download/v{latest_version}");

    download(&format!("{base_url}/{artifact_name}.tar.gz"), &tar_path)?;
    download(&format!("{base_url}/{artifact_name}.sha256"), &sha_path)?;

    pb.set_message("Verifying checksum...");
    let sha_content = std::fs::read_to_string(&sha_path).context("Failed to read checksum file")?;
    verify_checksum(&tar_path, &sha_content)?;

    pb.set_message("Installing...");
    let extract_dir = tmp.path().join("extract");
    std::fs::create_dir(&extract_dir)?;
    extract_tar(&tar_path, &extract_dir)?;

    let new_binary = extract_dir.join("workmux");
    if !new_binary.exists() {
        bail!("Extracted archive does not contain 'workmux' binary");
    }

    replace_binary(&new_binary, current_exe)?;

    Ok(format!(
        "Updated workmux v{CURRENT_VERSION} -> v{latest_version}"
    ))
}

pub fn run() -> Result<()> {
    let current_exe =
        std::env::current_exe().context("Could not determine current executable path")?;

    // Guard: Homebrew-managed installs (canonicalize to resolve symlinks)
    let canonical_exe = std::fs::canonicalize(&current_exe).unwrap_or(current_exe.clone());
    if is_homebrew_install(&canonical_exe) {
        bail!("workmux is managed by Homebrew. Run `brew upgrade workmux` instead.");
    }

    let platform = platform_suffix()?;
    let artifact_name = format!("workmux-{platform}");

    let pb = indicatif::ProgressBar::new_spinner();
    pb.set_style(
        indicatif::ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    pb.enable_steady_tick(std::time::Duration::from_millis(120));
    pb.set_message("Checking for updates...");

    match do_update(&pb, &artifact_name, &current_exe) {
        Ok(msg) => {
            pb.finish_with_message(format!("✔ {msg}"));
            Ok(())
        }
        Err(e) => {
            pb.finish_with_message("✘ Update failed".to_string());
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_suffix_current() {
        // Should succeed on any supported CI/dev platform
        let suffix = platform_suffix().unwrap();
        assert!(["darwin-arm64", "darwin-amd64", "linux-amd64", "linux-arm64"].contains(&suffix));
    }

    #[test]
    fn test_is_homebrew_cellar() {
        assert!(is_homebrew_install(std::path::Path::new(
            "/opt/homebrew/Cellar/workmux/0.1.124/bin/workmux"
        )));
    }

    #[test]
    fn test_is_homebrew_prefix() {
        assert!(is_homebrew_install(std::path::Path::new(
            "/usr/local/Cellar/workmux/0.1.124/bin/workmux"
        )));
    }

    #[test]
    fn test_is_not_homebrew_local_bin() {
        assert!(!is_homebrew_install(std::path::Path::new(
            "/usr/local/bin/workmux"
        )));
    }

    #[test]
    fn test_is_not_homebrew_home() {
        assert!(!is_homebrew_install(std::path::Path::new(
            "/home/user/.local/bin/workmux"
        )));
    }
}
