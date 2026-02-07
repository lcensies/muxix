//! Toolchain detection and command wrapping for Nix/Devbox environments.

use std::path::Path;

use crate::config::ToolchainMode;

/// Detected toolchain type in a project directory.
#[derive(Debug, Clone, PartialEq)]
pub enum DetectedToolchain {
    Devbox,
    Flake,
    None,
}

/// Detect which toolchain config file exists in the given directory.
/// devbox.json takes priority over flake.nix if both exist.
pub fn detect_toolchain(dir: &Path) -> DetectedToolchain {
    if dir.join("devbox.json").exists() {
        DetectedToolchain::Devbox
    } else if dir.join("flake.nix").exists() {
        DetectedToolchain::Flake
    } else {
        DetectedToolchain::None
    }
}

/// Resolve the effective toolchain based on config mode and detection.
pub fn resolve_toolchain(mode: &ToolchainMode, dir: &Path) -> DetectedToolchain {
    match mode {
        ToolchainMode::Off => DetectedToolchain::None,
        ToolchainMode::Devbox => DetectedToolchain::Devbox,
        ToolchainMode::Flake => DetectedToolchain::Flake,
        ToolchainMode::Auto => detect_toolchain(dir),
    }
}

fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Wrap a command string to run inside the appropriate toolchain environment.
/// Returns the original command unchanged if no toolchain is active.
///
/// For Devbox, generates a shell wrapper that:
/// 1. Hashes devbox.json + devbox.lock to compute a content-addressable cache key
/// 2. Creates a shared cache directory inside the VM (~/.cache/workmux/devbox/<hash>/)
/// 3. Copies config files there if not already present
/// 4. Runs `devbox run -c <cache-dir>` so all worktrees with the same config share
///    one .devbox/ environment, avoiding expensive re-initialization per worktree
pub fn wrap_command(command: &str, toolchain: &DetectedToolchain) -> String {
    match toolchain {
        DetectedToolchain::Devbox => {
            let escaped = shell_escape(command);
            // Shell wrapper that bootstraps a content-addressable devbox cache.
            // The hash is computed at runtime inside the VM from the mounted
            // devbox.json + devbox.lock, so config changes automatically create
            // a new cache entry.
            //
            // The command is passed as a single string (not via bash -c) because
            // devbox internally joins all args with spaces before passing to
            // `sh -c`, which breaks multi-word bash -c arguments. Passing a
            // single string preserves the quoting and ensures cd runs before
            // any command substitutions like $(cat ...).
            format!(
                concat!(
                    "_WM_CWD=\"$PWD\"; ",
                    "_WM_HASH=$(cat devbox.json devbox.lock 2>/dev/null | (md5sum 2>/dev/null || md5 -q) | cut -d\" \" -f1); ",
                    "_WM_CACHE=\"$HOME/.cache/workmux/devbox/$_WM_HASH\"; ",
                    "if [ ! -f \"$_WM_CACHE/devbox.json\" ]; then ",
                    "mkdir -p \"$_WM_CACHE\" && ",
                    "cp devbox.json \"$_WM_CACHE/\" && ",
                    "{{ [ ! -f devbox.lock ] || cp devbox.lock \"$_WM_CACHE/\"; }}; ",
                    "fi; ",
                    "export _WM_CWD; ",
                    "devbox run -c \"$_WM_CACHE\" -- 'cd \"$_WM_CWD\" && {}'"
                ),
                escaped
            )
        }
        DetectedToolchain::Flake => {
            let escaped = shell_escape(command);
            format!("nix develop --command bash -c '{}'", escaped)
        }
        DetectedToolchain::None => command.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_devbox() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("devbox.json"), "{}").unwrap();
        assert_eq!(detect_toolchain(dir.path()), DetectedToolchain::Devbox);
    }

    #[test]
    fn test_detect_flake() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("flake.nix"), "{}").unwrap();
        assert_eq!(detect_toolchain(dir.path()), DetectedToolchain::Flake);
    }

    #[test]
    fn test_detect_none() {
        let dir = TempDir::new().unwrap();
        assert_eq!(detect_toolchain(dir.path()), DetectedToolchain::None);
    }

    #[test]
    fn test_devbox_priority_over_flake() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("devbox.json"), "{}").unwrap();
        std::fs::write(dir.path().join("flake.nix"), "{}").unwrap();
        assert_eq!(detect_toolchain(dir.path()), DetectedToolchain::Devbox);
    }

    #[test]
    fn test_resolve_off_ignores_files() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("devbox.json"), "{}").unwrap();
        assert_eq!(
            resolve_toolchain(&ToolchainMode::Off, dir.path()),
            DetectedToolchain::None
        );
    }

    #[test]
    fn test_resolve_forced_devbox() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            resolve_toolchain(&ToolchainMode::Devbox, dir.path()),
            DetectedToolchain::Devbox
        );
    }

    #[test]
    fn test_resolve_forced_flake() {
        let dir = TempDir::new().unwrap();
        assert_eq!(
            resolve_toolchain(&ToolchainMode::Flake, dir.path()),
            DetectedToolchain::Flake
        );
    }

    #[test]
    fn test_resolve_auto_delegates_to_detect() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("flake.nix"), "{}").unwrap();
        assert_eq!(
            resolve_toolchain(&ToolchainMode::Auto, dir.path()),
            DetectedToolchain::Flake
        );
    }

    #[test]
    fn test_wrap_devbox_uses_cache() {
        let wrapped = wrap_command("claude --help", &DetectedToolchain::Devbox);
        // Should save working directory and restore it inside devbox
        assert!(wrapped.contains("_WM_CWD=\"$PWD\""));
        assert!(wrapped.contains("export _WM_CWD"));
        assert!(wrapped.contains("cd \"$_WM_CWD\""));
        // Should hash config files for cache key (portable: md5sum || md5)
        assert!(wrapped.contains("md5sum"));
        assert!(wrapped.contains("md5 -q"));
        assert!(wrapped.contains("devbox.json"));
        assert!(wrapped.contains("devbox.lock"));
        // Should use shared cache dir
        assert!(wrapped.contains(".cache/workmux/devbox/"));
        // Should copy config to cache
        assert!(wrapped.contains("cp devbox.json"));
        // Should use -c flag pointing to cache
        assert!(wrapped.contains("devbox run -c"));
        // Should contain the escaped command
        assert!(wrapped.contains("claude --help"));
    }

    #[test]
    fn test_wrap_devbox_escapes_quotes() {
        let wrapped = wrap_command("echo 'hello'", &DetectedToolchain::Devbox);
        assert!(wrapped.contains(r"echo '\''hello'\''"));
    }

    #[test]
    fn test_wrap_flake() {
        assert_eq!(
            wrap_command("claude --help", &DetectedToolchain::Flake),
            "nix develop --command bash -c 'claude --help'"
        );
    }

    #[test]
    fn test_wrap_flake_escapes_single_quotes() {
        let cmd = "echo 'hello world'";
        let wrapped = wrap_command(cmd, &DetectedToolchain::Flake);
        assert_eq!(
            wrapped,
            r#"nix develop --command bash -c 'echo '\''hello world'\'''"#
        );
    }

    #[test]
    fn test_wrap_none_passthrough() {
        assert_eq!(
            wrap_command("claude --help", &DetectedToolchain::None),
            "claude --help"
        );
    }
}
