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

/// Wrap a command string to run inside the appropriate toolchain environment.
/// Returns the original command unchanged if no toolchain is active.
pub fn wrap_command(command: &str, toolchain: &DetectedToolchain) -> String {
    match toolchain {
        DetectedToolchain::Devbox => {
            let escaped = command.replace('\'', "'\\''");
            format!("devbox run -- bash -lc '{}'", escaped)
        }
        DetectedToolchain::Flake => {
            let escaped = command.replace('\'', "'\\''");
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
    fn test_wrap_devbox() {
        assert_eq!(
            wrap_command("claude --help", &DetectedToolchain::Devbox),
            "devbox run -- bash -lc 'claude --help'"
        );
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
