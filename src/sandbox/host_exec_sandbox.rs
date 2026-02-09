//! OS-native sandboxing for host-exec child processes.
//!
//! Restricts file and process access for commands executed on the host on
//! behalf of a sandboxed guest. Uses `sandbox-exec` (Seatbelt) on macOS
//! and `bwrap` (Bubblewrap) on Linux.
//!
//! The goal is defense-in-depth: even if a guest writes a malicious build
//! file (justfile, build.rs, package.json), the host-exec child process
//! cannot read sensitive files (~/.ssh, ~/.aws) or write outside the
//! worktree and toolchain caches.

use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use tracing::debug;

/// Directories under $HOME that are denied read access.
/// These contain credentials, keys, and other secrets.
const DENY_READ_DIRS: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".azure",
    ".config/gcloud",
    ".config/workmux",
    ".docker",
];

/// Files under $HOME that are denied read access.
/// Separate from DENY_READ_DIRS because Linux bwrap needs different
/// handling (bind /dev/null over files, tmpfs over directories).
const DENY_READ_FILES: &[&str] = &[
    ".npmrc",           // can contain auth tokens
    ".pypirc",          // can contain auth tokens
    ".netrc",           // network credentials
    ".gem/credentials", // rubygems auth
];

/// macOS-specific deny paths (absolute, not relative to HOME).
#[cfg(target_os = "macos")]
const DENY_READ_PATHS_MACOS: &[&str] = &[
    "Library/Keychains",
    "Library/Cookies",
    "Library/Application Support/Google/Chrome",
    "Library/Application Support/Firefox",
];

/// Directories under $HOME that are allowed write access (caches, toolchain state).
/// Everything else under $HOME is write-denied.
const ALLOW_WRITE_DIRS: &[&str] = &[
    ".cache",
    ".cargo",
    ".rustup",
    ".npm",
    ".local/state",
    ".local/share/devbox",
];

/// macOS-specific write-allowed paths.
#[cfg(target_os = "macos")]
const ALLOW_WRITE_DIRS_MACOS: &[&str] = &["Library/Caches", "Library/Logs"];

/// Spawn a command inside an OS-native sandbox.
///
/// On macOS, uses `sandbox-exec`. On Linux, uses `bwrap` resolved from
/// trusted system paths. When `allow_unsandboxed` is true, skips the
/// filesystem sandbox on either platform.
pub fn spawn_sandboxed(
    program: &str,
    args: &[String],
    worktree: &Path,
    envs: &HashMap<String, String>,
    allow_unsandboxed: bool,
) -> Result<Child> {
    if allow_unsandboxed {
        tracing::warn!(
            "dangerously_allow_unsandboxed_host_exec is set, skipping filesystem sandbox"
        );
        return spawn_unsandboxed(program, args, worktree, envs);
    }

    #[cfg(target_os = "macos")]
    {
        spawn_macos(program, args, worktree, envs)
    }

    #[cfg(target_os = "linux")]
    {
        spawn_linux(program, args, worktree, envs)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        tracing::warn!("host-exec sandboxing not supported on this OS, running unsandboxed");
        spawn_unsandboxed(program, args, worktree, envs)
    }
}

fn spawn_unsandboxed(
    program: &str,
    args: &[String],
    worktree: &Path,
    envs: &HashMap<String, String>,
) -> Result<Child> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    cmd.current_dir(worktree);
    cmd.env_clear();
    cmd.envs(envs);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.spawn().context("Failed to spawn command")
}

// ── macOS: sandbox-exec ─────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn spawn_macos(
    program: &str,
    args: &[String],
    worktree: &Path,
    envs: &HashMap<String, String>,
) -> Result<Child> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/var/empty".to_string());
    let worktree_str = worktree
        .to_str()
        .context("Worktree path is not valid UTF-8")?;

    let profile = generate_macos_profile();

    let mut cmd = Command::new("/usr/bin/sandbox-exec");
    // Use -D parameter substitution to inject paths safely (no string interpolation)
    cmd.arg("-p").arg(&profile);
    cmd.arg("-D").arg(format!("HOME_DIR={}", home));
    cmd.arg("-D").arg(format!("WORKTREE={}", worktree_str));
    cmd.arg(program);
    cmd.args(args);
    cmd.current_dir(worktree);
    cmd.env_clear();
    cmd.envs(envs);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    debug!(program, "spawning under sandbox-exec");
    cmd.spawn().context("Failed to spawn sandbox-exec")
}

/// Generate the macOS Seatbelt profile string.
///
/// Uses `(param ...)` references for HOME_DIR and WORKTREE so paths are
/// injected via `-D` flags rather than string interpolation. This prevents
/// profile injection via crafted paths.
#[cfg(target_os = "macos")]
fn generate_macos_profile() -> String {
    let mut profile = String::from("(version 1)\n(allow default)\n\n");

    // Deny reading sensitive directories and files under HOME
    profile.push_str("; Deny reading credentials and secrets\n");
    profile.push_str("(deny file-read* (with no-report)\n");
    for dir in DENY_READ_DIRS {
        profile.push_str(&format!(
            "    (subpath (string-append (param \"HOME_DIR\") \"/{}\" ))\n",
            dir
        ));
    }
    for file in DENY_READ_FILES {
        // Use literal for files (subpath works for both files and dirs in Seatbelt)
        profile.push_str(&format!(
            "    (subpath (string-append (param \"HOME_DIR\") \"/{}\" ))\n",
            file
        ));
    }
    for dir in DENY_READ_PATHS_MACOS {
        profile.push_str(&format!(
            "    (subpath (string-append (param \"HOME_DIR\") \"/{}\" ))\n",
            dir
        ));
    }
    profile.push_str(")\n\n");

    // Deny writing to HOME (broad)
    profile.push_str("; Deny writing to HOME except allowed caches\n");
    profile.push_str("(deny file-write* (with no-report)\n");
    profile.push_str("    (subpath (param \"HOME_DIR\"))\n");
    profile.push_str(")\n\n");

    // Allow writing to worktree
    profile.push_str("; Allow full access to worktree\n");
    profile.push_str("(allow file-read* file-write*\n");
    profile.push_str("    (subpath (param \"WORKTREE\"))\n");
    profile.push_str(")\n\n");

    // Allow writing to cache/toolchain dirs
    profile.push_str("; Allow writing to caches and toolchains\n");
    profile.push_str("(allow file-write*\n");
    for dir in ALLOW_WRITE_DIRS {
        profile.push_str(&format!(
            "    (subpath (string-append (param \"HOME_DIR\") \"/{}\" ))\n",
            dir
        ));
    }
    for dir in ALLOW_WRITE_DIRS_MACOS {
        profile.push_str(&format!(
            "    (subpath (string-append (param \"HOME_DIR\") \"/{}\" ))\n",
            dir
        ));
    }
    profile.push_str(")\n\n");

    // Allow writing to temp dirs
    profile.push_str("; Allow temp directories\n");
    profile.push_str("(allow file-read* file-write*\n");
    profile.push_str("    (subpath \"/tmp\")\n");
    profile.push_str("    (subpath \"/private/tmp\")\n");
    profile.push_str("    (subpath \"/var/folders\")\n");
    profile.push_str(")\n\n");

    // Allow nix store access
    profile.push_str("; Allow nix store (read-only by default, allow write for installs)\n");
    profile.push_str("(allow file-read* file-write* (subpath \"/nix\"))\n");

    profile
}

// ── Linux: bwrap ────────────────────────────────────────────────────────

/// Trusted system paths where bwrap may be installed.
/// We avoid PATH resolution to prevent hijacking via writable directories.
#[cfg(target_os = "linux")]
const TRUSTED_BWRAP_PATHS: &[&str] = &[
    "/usr/bin/bwrap",
    "/usr/sbin/bwrap",
    "/usr/local/bin/bwrap",
    "/run/current-system/sw/bin/bwrap",     // NixOS
    "/home/linuxbrew/.linuxbrew/bin/bwrap", // Homebrew on Linux
];

/// Find bwrap at a trusted absolute path. Returns the first match.
#[cfg(target_os = "linux")]
fn find_bwrap() -> Option<&'static str> {
    TRUSTED_BWRAP_PATHS
        .iter()
        .find(|p| Path::new(p).is_file())
        .copied()
}

#[cfg(target_os = "linux")]
fn spawn_linux(
    program: &str,
    args: &[String],
    worktree: &Path,
    envs: &HashMap<String, String>,
) -> Result<Child> {
    if let Some(bwrap_path) = find_bwrap() {
        spawn_bwrap(bwrap_path, program, args, worktree, envs)
    } else {
        anyhow::bail!(
            "bwrap (bubblewrap) not found at any trusted path ({}). \
            Install bubblewrap or set sandbox.dangerously_allow_unsandboxed_host_exec: true \
            in your global config to run without filesystem isolation",
            TRUSTED_BWRAP_PATHS.join(", ")
        )
    }
}

#[cfg(target_os = "linux")]
fn spawn_bwrap(
    bwrap_path: &str,
    program: &str,
    args: &[String],
    worktree: &Path,
    envs: &HashMap<String, String>,
) -> Result<Child> {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/var/empty".to_string());
    let home_path = Path::new(&home);

    let mut cmd = Command::new(bwrap_path);

    // Read-only root filesystem
    cmd.args(["--ro-bind", "/", "/"]);
    cmd.args(["--dev", "/dev"]);
    cmd.args(["--proc", "/proc"]);
    cmd.args(["--tmpfs", "/tmp"]);

    // Writable worktree
    let wt = worktree
        .to_str()
        .context("Worktree path is not valid UTF-8")?;
    cmd.args(["--bind", wt, wt]);

    // Hide secret directories behind tmpfs
    for dir in DENY_READ_DIRS {
        let path = home_path.join(dir);
        if path.exists() {
            if let Some(s) = path.to_str() {
                cmd.args(["--tmpfs", s]);
            }
        }
    }

    // Hide secret files by binding /dev/null over them
    for file in DENY_READ_FILES {
        let path = home_path.join(file);
        if path.is_file() {
            if let Some(s) = path.to_str() {
                cmd.args(["--ro-bind", "/dev/null", s]);
            }
        }
    }

    // Writable caches -- create dirs if needed so bwrap can bind-mount them
    // (the root is read-only, so the process can't create them itself)
    for dir in ALLOW_WRITE_DIRS {
        let path = home_path.join(dir);
        if !path.exists() {
            if let Err(e) = std::fs::create_dir_all(&path) {
                debug!(?path, error = %e, "failed to create cache dir for bwrap binding");
                continue;
            }
        }
        if let Some(s) = path.to_str() {
            cmd.args(["--bind", s, s]);
        }
    }

    // Allow network (required for package managers)
    cmd.arg("--share-net");

    // Execute the command
    cmd.arg("--");
    cmd.arg(program);
    cmd.args(args);

    cmd.current_dir(worktree);
    cmd.env_clear();
    cmd.envs(envs);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    debug!(program, "spawning under bwrap");
    cmd.spawn().context("Failed to spawn bwrap")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deny_read_dirs_are_valid() {
        for dir in DENY_READ_DIRS {
            assert!(!dir.starts_with('/'), "should be relative: {}", dir);
            assert!(!dir.contains(".."), "no traversal: {}", dir);
            assert!(!dir.is_empty(), "no empty entries");
        }
    }

    #[test]
    fn test_deny_read_files_are_valid() {
        for file in DENY_READ_FILES {
            assert!(!file.starts_with('/'), "should be relative: {}", file);
            assert!(!file.contains(".."), "no traversal: {}", file);
            assert!(!file.is_empty(), "no empty entries");
        }
    }

    #[test]
    fn test_allow_write_dirs_are_valid() {
        for dir in ALLOW_WRITE_DIRS {
            assert!(!dir.starts_with('/'), "should be relative: {}", dir);
            assert!(!dir.contains(".."), "no traversal: {}", dir);
        }
    }

    #[test]
    fn test_deny_and_allow_dont_overlap() {
        // No entry should be both denied for reading and allowed for writing
        for deny in DENY_READ_DIRS.iter().chain(DENY_READ_FILES.iter()) {
            for allow in ALLOW_WRITE_DIRS {
                assert_ne!(
                    *deny, *allow,
                    "overlap between deny-read and allow-write: {}",
                    deny
                );
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_profile_uses_params() {
        let profile = generate_macos_profile();
        // Must use param references, not hardcoded paths
        assert!(profile.contains("(param \"HOME_DIR\")"));
        assert!(profile.contains("(param \"WORKTREE\")"));
        // Must not contain literal home paths
        assert!(!profile.contains("/Users/"));
        assert!(!profile.contains("/home/"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_profile_denies_ssh() {
        let profile = generate_macos_profile();
        assert!(profile.contains(".ssh"));
        assert!(profile.contains(".aws"));
        assert!(profile.contains(".gnupg"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_macos_profile_allows_worktree_and_caches() {
        let profile = generate_macos_profile();
        assert!(profile.contains("WORKTREE"));
        assert!(profile.contains(".cache"));
        assert!(profile.contains(".cargo"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_trusted_bwrap_paths_are_absolute() {
        for path in TRUSTED_BWRAP_PATHS {
            assert!(
                path.starts_with('/'),
                "trusted bwrap path must be absolute: {}",
                path
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_trusted_bwrap_paths_end_with_bwrap() {
        for path in TRUSTED_BWRAP_PATHS {
            assert!(
                path.ends_with("/bwrap"),
                "trusted bwrap path must end with /bwrap: {}",
                path
            );
        }
    }
}
