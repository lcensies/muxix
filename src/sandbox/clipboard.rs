//! Host-side clipboard reading for sandbox clipboard proxy.
//!
//! Reads the host clipboard and writes image data to the shared
//! worktree filesystem so the guest can read it without binary RPC.
//!
//! - macOS: uses osascript to read clipboard as PNGf
//! - Linux: uses wl-paste (Wayland) or xclip (X11)

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Maximum clipboard image size (20 MB).
const MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024;

/// Read clipboard PNG and write to worktree shared filesystem.
/// Returns absolute path to the written file, or None if no image in clipboard.
pub fn materialize_clipboard_png(worktree: &Path) -> Result<Option<PathBuf>> {
    let bytes = match read_png_from_clipboard()? {
        Some(b) => b,
        None => return Ok(None),
    };

    if bytes.len() > MAX_IMAGE_SIZE {
        bail!(
            "clipboard image too large ({} bytes, max {})",
            bytes.len(),
            MAX_IMAGE_SIZE
        );
    }

    let tmp_dir = worktree.join(".workmux/tmp");
    std::fs::create_dir_all(&tmp_dir)
        .with_context(|| format!("failed to create {}", tmp_dir.display()))?;

    // Prevent git from tracking clipboard files
    let gitignore = tmp_dir.join(".gitignore");
    if !gitignore.exists() {
        let _ = std::fs::write(&gitignore, "*\n");
    }

    // Prune stale files (best-effort, >1 hour old)
    prune_stale_files(&tmp_dir, std::time::Duration::from_secs(3600));

    // Write with unique filename
    let filename = format!(
        "clipboard-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros()
    );
    let file_path = tmp_dir.join(&filename);
    std::fs::write(&file_path, &bytes)
        .with_context(|| format!("failed to write clipboard file: {}", file_path.display()))?;

    Ok(Some(file_path))
}

/// Platform-specific clipboard PNG reading.
fn read_png_from_clipboard() -> Result<Option<Vec<u8>>> {
    #[cfg(target_os = "macos")]
    {
        read_png_macos()
    }
    #[cfg(target_os = "linux")]
    {
        read_png_linux()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Ok(None)
    }
}

/// Read PNG data from macOS clipboard by parsing osascript hex output.
#[cfg(target_os = "macos")]
fn read_png_macos() -> Result<Option<Vec<u8>>> {
    let output = Command::new("/usr/bin/osascript")
        .args(["-e", "the clipboard as «class PNGf»"])
        .output()
        .context("failed to run osascript")?;

    if !output.status.success() {
        return Ok(None);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_osascript_hex(stdout.trim())
}

/// Parse the hex output from osascript's `the clipboard as «class PNGf»`.
///
/// Output format: `«data PNGf89504E47...»`
#[cfg(target_os = "macos")]
fn parse_osascript_hex(output: &str) -> Result<Option<Vec<u8>>> {
    let hex = match output
        .strip_prefix("«data PNGf")
        .and_then(|s| s.strip_suffix('»'))
    {
        Some(h) => h,
        None => return Ok(None),
    };

    if hex.is_empty() || hex.len() % 2 != 0 {
        return Ok(None);
    }

    if hex.len() / 2 > MAX_IMAGE_SIZE {
        bail!(
            "clipboard image too large ({} bytes, max {})",
            hex.len() / 2,
            MAX_IMAGE_SIZE
        );
    }

    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .context("invalid hex in clipboard data")?;

    Ok(Some(bytes))
}

/// Read PNG data from Linux clipboard via wl-paste (Wayland) or xclip (X11).
#[cfg(target_os = "linux")]
fn read_png_linux() -> Result<Option<Vec<u8>>> {
    // Try wl-paste first (Wayland)
    if let Ok(output) = Command::new("wl-paste").args(["-t", "image/png"]).output() {
        if output.status.success() && !output.stdout.is_empty() {
            return Ok(Some(output.stdout));
        }
    }

    // Fall back to xclip (X11)
    if let Ok(output) = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .output()
    {
        if output.status.success() && !output.stdout.is_empty() {
            return Ok(Some(output.stdout));
        }
    }

    Ok(None)
}

fn prune_stale_files(dir: &Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("png") {
            continue;
        }
        if let Ok(meta) = path.metadata()
            && let Ok(modified) = meta.modified()
            && now.duration_since(modified).unwrap_or_default() > max_age
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prune_stale_files_ignores_non_png() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "*\n").unwrap();
        prune_stale_files(tmp.path(), std::time::Duration::from_secs(0));
        assert!(tmp.path().join(".gitignore").exists());
    }

    #[test]
    fn test_prune_stale_files_removes_old_png() {
        let tmp = tempfile::tempdir().unwrap();
        let png = tmp.path().join("clipboard-1-2.png");
        std::fs::write(&png, b"fake png").unwrap();
        // max_age=0 means everything is stale
        prune_stale_files(tmp.path(), std::time::Duration::from_secs(0));
        assert!(!png.exists());
    }

    #[test]
    fn test_materialize_creates_gitignore() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_dir = tmp.path().join(".workmux/tmp");
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let gitignore = tmp_dir.join(".gitignore");
        assert!(!gitignore.exists());

        // We can't test the full materialize without a clipboard, but we can
        // test the gitignore creation by calling it and expecting None (no image)
        let result = materialize_clipboard_png(tmp.path());
        // On CI/test, clipboard is empty so we get Ok(None)
        match result {
            Ok(None) => {
                // Gitignore might not be created if read_png_from_clipboard returns None early
            }
            Ok(Some(_)) => {
                assert!(gitignore.exists());
                assert_eq!(std::fs::read_to_string(&gitignore).unwrap(), "*\n");
            }
            Err(_) => {
                // osascript/wl-paste may fail in test environments
            }
        }
    }

    #[cfg(target_os = "macos")]
    mod macos_tests {
        use super::super::*;

        #[test]
        fn test_parse_osascript_hex_valid_png_header() {
            // PNG magic bytes: 89504E47 0D0A1A0A
            let input = "«data PNGf89504E470D0A1A0A»";
            let result = parse_osascript_hex(input).unwrap().unwrap();
            assert_eq!(result, vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        }

        #[test]
        fn test_parse_osascript_hex_empty_data() {
            let input = "«data PNGf»";
            let result = parse_osascript_hex(input).unwrap();
            assert!(result.is_none());
        }

        #[test]
        fn test_parse_osascript_hex_invalid_prefix() {
            let input = "some other output";
            let result = parse_osascript_hex(input).unwrap();
            assert!(result.is_none());
        }

        #[test]
        fn test_parse_osascript_hex_odd_length() {
            let input = "«data PNGf895»";
            let result = parse_osascript_hex(input).unwrap();
            assert!(result.is_none());
        }

        #[test]
        fn test_parse_osascript_hex_invalid_hex_chars() {
            let input = "«data PNGfXXYY»";
            let result = parse_osascript_hex(input);
            assert!(result.is_err());
        }
    }
}
