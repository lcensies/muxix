//! Host-exec shim creation for sandbox guests (Lima VMs and containers).
//!
//! Creates a directory of symlinks that intercept configured command names
//! and route them to `workmux host-exec`.

use anyhow::{Context, Result};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

/// Commands that are always available as host-exec shims, regardless of
/// user `host_commands` config. These are system-level commands that sandbox
/// guests need to proxy to the host (e.g., `afplay` for macOS sound).
pub const BUILTIN_HOST_COMMANDS: &[&str] = &["afplay"];

/// Merge built-in host commands with user-configured ones, deduplicating.
pub fn effective_host_commands(user_commands: &[String]) -> Vec<String> {
    let mut commands: Vec<String> = BUILTIN_HOST_COMMANDS
        .iter()
        .map(|s| s.to_string())
        .collect();
    for cmd in user_commands {
        if !commands.iter().any(|c| c == cmd) {
            commands.push(cmd.clone());
        }
    }
    commands
}

/// Create a shim directory with a dispatcher script and command symlinks.
///
/// The directory is created under the VM's state dir (which is mounted
/// into the guest at ~/.workmux-state/). Returns the guest-visible path
/// to prepend to PATH.
///
/// Layout:
///   <state_dir>/shims/bin/_shim    (dispatcher script)
///   <state_dir>/shims/bin/just     -> _shim
///   <state_dir>/shims/bin/cargo    -> _shim
pub fn create_shim_directory(state_dir: &Path, commands: &[String]) -> Result<PathBuf> {
    let shim_bin = state_dir.join("shims/bin");
    fs::create_dir_all(&shim_bin)
        .with_context(|| format!("Failed to create shim dir: {}", shim_bin.display()))?;

    // Write dispatcher script
    let dispatcher = shim_bin.join("_shim");
    fs::write(
        &dispatcher,
        "#!/bin/sh\nexec workmux host-exec \"$(basename \"$0\")\" \"$@\"\n",
    )
    .context("Failed to write shim dispatcher")?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dispatcher, fs::Permissions::from_mode(0o755))?;
    }

    // Create symlinks for each command
    for cmd in commands {
        // Validate: no path separators allowed
        if cmd.contains('/') || cmd.contains('\\') || cmd.is_empty() {
            tracing::warn!(command = cmd, "skipping invalid host_command name");
            continue;
        }

        let link = shim_bin.join(cmd);
        // Atomic: create temp symlink and rename into place.
        // Safe under concurrent supervisors sharing the same VM.
        let tmp = shim_bin.join(format!(".{}.tmp", cmd));
        let _ = fs::remove_file(&tmp);
        symlink("_shim", &tmp)
            .with_context(|| format!("Failed to create temp shim symlink for: {}", cmd))?;
        fs::rename(&tmp, &link)
            .with_context(|| format!("Failed to rename shim symlink for: {}", cmd))?;
    }

    Ok(shim_bin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_shim_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let commands = vec!["just".to_string(), "cargo".to_string(), "npm".to_string()];

        let shim_bin = create_shim_directory(tmp.path(), &commands).unwrap();

        // Dispatcher exists and is executable
        let dispatcher = shim_bin.join("_shim");
        assert!(dispatcher.exists());
        let content = std::fs::read_to_string(&dispatcher).unwrap();
        assert!(content.contains("workmux host-exec"));

        // Symlinks exist
        for cmd in &commands {
            let link = shim_bin.join(cmd);
            assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
            assert_eq!(std::fs::read_link(&link).unwrap(), PathBuf::from("_shim"));
        }
    }

    #[test]
    fn test_create_shim_directory_skips_invalid() {
        let tmp = tempfile::tempdir().unwrap();
        let commands = vec!["valid".to_string(), "/bin/evil".to_string(), "".to_string()];

        let shim_bin = create_shim_directory(tmp.path(), &commands).unwrap();
        assert!(shim_bin.join("valid").exists());
        assert!(!shim_bin.join("/bin/evil").exists());
    }

    #[test]
    fn test_effective_host_commands_includes_builtins() {
        let result = effective_host_commands(&[]);
        assert!(result.contains(&"afplay".to_string()));
    }

    #[test]
    fn test_effective_host_commands_merges_user() {
        let result = effective_host_commands(&["just".to_string(), "cargo".to_string()]);
        assert_eq!(result, vec!["afplay", "just", "cargo"]);
    }

    #[test]
    fn test_effective_host_commands_deduplicates() {
        let result = effective_host_commands(&["afplay".to_string(), "just".to_string()]);
        assert_eq!(result, vec!["afplay", "just"]);
    }

    #[test]
    fn test_create_shim_directory_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let commands = vec!["just".to_string()];

        create_shim_directory(tmp.path(), &commands).unwrap();
        // Running again should not error
        create_shim_directory(tmp.path(), &commands).unwrap();

        assert!(tmp.path().join("shims/bin/just").exists());
    }
}
