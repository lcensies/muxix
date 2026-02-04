//! Docker/Podman container sandbox implementation.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::config::{SandboxConfig, SandboxRuntime};

/// Embedded Dockerfile for building sandbox image.
/// Uses debian:bookworm-slim for glibc compatibility with host-built binaries.
const SANDBOX_DOCKERFILE: &str = r#"FROM debian:bookworm-slim

# Install dependencies for Claude Code + git operations
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl \
    ca-certificates \
    git \
    && rm -rf /var/lib/apt/lists/*

# Install Claude Code
RUN curl -fsSL https://claude.ai/install.sh | bash

# Copy workmux binary from build context
COPY workmux /usr/local/bin/workmux
RUN chmod +x /usr/local/bin/workmux

# Add claude to PATH
ENV PATH="/root/.claude/local/bin:${PATH}"
"#;

/// Sandbox-specific config paths on host.
/// These are separate from host CLI config to avoid confusion.
pub struct SandboxPaths {
    /// ~/.claude-sandbox.json - main config/auth file
    pub config_file: PathBuf,
    /// ~/.claude-sandbox/ - settings directory
    pub config_dir: PathBuf,
}

impl SandboxPaths {
    pub fn new() -> Option<Self> {
        let home = home::home_dir()?;
        Some(Self {
            config_file: home.join(".claude-sandbox.json"),
            config_dir: home.join(".claude-sandbox"),
        })
    }
}

/// Ensure sandbox config directories exist on host.
pub fn ensure_sandbox_config_dirs() -> Result<SandboxPaths> {
    let paths = SandboxPaths::new().context("Could not determine home directory")?;

    // Create empty config file if it doesn't exist
    if !paths.config_file.exists() {
        std::fs::write(&paths.config_file, "{}")
            .with_context(|| format!("Failed to create {}", paths.config_file.display()))?;
    }

    // Create config directory if it doesn't exist
    if !paths.config_dir.exists() {
        std::fs::create_dir_all(&paths.config_dir)
            .with_context(|| format!("Failed to create {}", paths.config_dir.display()))?;
    }

    Ok(paths)
}

/// Run interactive auth flow in container.
/// Mounts sandbox config paths read-write so auth persists.
pub fn run_auth(config: &SandboxConfig) -> Result<()> {
    let paths = ensure_sandbox_config_dirs()?;
    let runtime = match config.runtime() {
        SandboxRuntime::Podman => "podman",
        SandboxRuntime::Docker => "docker",
    };
    let image = config.resolved_image();

    let status = Command::new(runtime)
        .args([
            "run",
            "-it",
            "--rm",
            // Mount sandbox-specific config (read-write for auth)
            "--mount",
            &format!(
                "type=bind,source={},target=/tmp/.claude.json",
                paths.config_file.display()
            ),
            "--mount",
            &format!(
                "type=bind,source={},target=/tmp/.claude",
                paths.config_dir.display()
            ),
            // Set HOME to /tmp where config is mounted
            "--env",
            "HOME=/tmp",
            // PATH for claude binary (include Claude Code install location)
            "--env",
            "PATH=/root/.claude/local/bin:/root/.local/bin:/usr/local/bin:/usr/bin:/bin",
            image,
            "claude",
        ])
        .status()
        .context("Failed to run container")?;

    if !status.success() {
        anyhow::bail!("Auth container exited with status: {}", status);
    }

    Ok(())
}

/// Build the sandbox container image.
///
/// Creates a minimal build context with the current workmux binary and an
/// embedded Dockerfile, then runs docker/podman build.
///
/// # Arguments
/// * `config` - Sandbox configuration
/// * `force` - If true, skip OS compatibility check
pub fn build_image(config: &SandboxConfig, force: bool) -> Result<()> {
    // Check OS compatibility - Linux binaries won't run on other OSes
    if !cfg!(target_os = "linux") && !force {
        anyhow::bail!(
            "Cannot build sandbox image on non-Linux OS.\n\
             The workmux binary in your image would be incompatible with the Linux container.\n\n\
             Options:\n\
             1. Build on a Linux machine\n\
             2. Use --force to build anyway (image will lack working workmux)\n\
             3. Manually build an image with workmux installed from releases"
        );
    }

    let runtime = match config.runtime() {
        SandboxRuntime::Podman => "podman",
        SandboxRuntime::Docker => "docker",
    };

    let image_name = config.resolved_image();

    // Create temporary build context
    let temp_dir = tempfile::Builder::new()
        .prefix("workmux-sandbox-build-")
        .tempdir()
        .context("Failed to create temporary build directory")?;
    let context_path = temp_dir.path();

    // Copy current workmux binary to build context
    let current_exe =
        std::env::current_exe().context("Failed to locate current workmux executable")?;
    let dest_exe = context_path.join("workmux");
    std::fs::copy(&current_exe, &dest_exe)
        .context("Failed to copy workmux binary to build context")?;

    // Write Dockerfile
    let dockerfile_path = context_path.join("Dockerfile");
    std::fs::write(&dockerfile_path, SANDBOX_DOCKERFILE).context("Failed to write Dockerfile")?;

    println!("Building image '{}' using {}...", image_name, runtime);

    // Run build
    let status = Command::new(runtime)
        .args(["build", "-t", image_name, "."])
        .current_dir(context_path)
        .status()
        .with_context(|| format!("Failed to run {} build", runtime))?;

    if !status.success() {
        anyhow::bail!("{} build failed with exit code: {}", runtime, status);
    }

    Ok(())
}

/// Wrap a command to run inside a Docker/Podman container.
///
/// Uses mirror mounting (project at same path) to preserve git worktree
/// references and terminal hyperlink compatibility.
///
/// Config mounts use sandbox-specific paths:
/// - ~/.claude-sandbox.json -> /root/.claude.json
/// - ~/.claude-sandbox/ -> /root/.claude/
///
/// # Arguments
/// * `command` - The command to run inside the container
/// * `config` - Sandbox configuration
/// * `worktree_root` - The root of the worktree (for mounting)
/// * `pane_cwd` - The working directory for the pane (may be a subdirectory)
pub fn wrap_for_container(
    command: &str,
    config: &SandboxConfig,
    worktree_root: &Path,
    pane_cwd: &Path,
) -> Result<String> {
    let runtime = match config.runtime() {
        SandboxRuntime::Podman => "podman",
        SandboxRuntime::Docker => "docker",
    };

    let image = config.resolved_image();
    let worktree_root_str = worktree_root.to_string_lossy();
    let pane_cwd_str = pane_cwd.to_string_lossy();

    // Get host UID:GID for permission handling
    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    let mut args = Vec::new();

    // Base command
    args.push(runtime.to_string());
    args.push("run".to_string());
    args.push("--rm".to_string());
    args.push("-it".to_string());

    // User mapping to prevent root-owned files
    args.push("--user".to_string());
    args.push(format!("{}:{}", uid, gid));

    // Mirror mount: worktree root at exact same path for git/terminal compatibility
    args.push("--mount".to_string());
    args.push(format!(
        "type=bind,source={},target={}",
        worktree_root_str, worktree_root_str
    ));

    // For git worktrees, also mount the main repo's .git directory.
    // Worktrees have a .git FILE (not directory) that points to the main repo.
    // Without this mount, git commands fail with "not a git repository".
    let git_path = worktree_root.join(".git");
    if git_path.is_file() {
        // Read the gitdir path from the .git file
        if let Ok(content) = std::fs::read_to_string(&git_path) {
            // Format: "gitdir: /path/to/main/.git/worktrees/name"
            if let Some(gitdir) = content.strip_prefix("gitdir: ") {
                let gitdir = gitdir.trim();
                // Mount the main .git directory (parent of worktrees/<name>)
                // e.g., /path/to/main/.git/worktrees/name -> /path/to/main/.git
                if let Some(main_git) = Path::new(gitdir).ancestors().nth(2) {
                    args.push("--mount".to_string());
                    args.push(format!(
                        "type=bind,source={},target={}",
                        main_git.display(),
                        main_git.display()
                    ));
                }
            }
        }
    }

    // Working directory (may be subdir of worktree for monorepos)
    args.push("--workdir".to_string());
    args.push(pane_cwd_str.to_string());

    // Use /tmp as HOME since we run as non-root user who can't write to /root
    args.push("--env".to_string());
    args.push("HOME=/tmp".to_string());

    // Mount sandbox-specific config to /tmp (matching HOME) for auth persistence
    if let Some(paths) = SandboxPaths::new() {
        if paths.config_file.exists() {
            args.push("--mount".to_string());
            args.push(format!(
                "type=bind,source={},target=/tmp/.claude.json",
                paths.config_file.display()
            ));
        }
        if paths.config_dir.exists() {
            args.push("--mount".to_string());
            args.push(format!(
                "type=bind,source={},target=/tmp/.claude",
                paths.config_dir.display()
            ));
        }
    }

    // Pass through environment variables
    for var in config.env_passthrough() {
        if std::env::var(var).is_ok() {
            args.push("--env".to_string());
            args.push(var.to_string());
        }
    }

    // PATH for agent binaries (include Claude Code install location)
    args.push("--env".to_string());
    args.push(
        "PATH=/root/.claude/local/bin:/root/.local/bin:/usr/local/bin:/usr/bin:/bin".to_string(),
    );

    // Image
    args.push(image.to_string());

    // Command wrapped in sh -c
    let escaped_command = command.replace('\'', "'\\''");
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(format!("'{}'", escaped_command));

    Ok(args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{SandboxConfig, SandboxRuntime};

    fn make_config() -> SandboxConfig {
        SandboxConfig {
            enabled: Some(true),
            runtime: Some(SandboxRuntime::Docker),
            image: Some("test-image:latest".to_string()),
            env_passthrough: Some(vec!["TEST_KEY".to_string()]),
            ..Default::default()
        }
    }

    #[test]
    fn test_wrap_basic_command() {
        let config = make_config();
        let result = wrap_for_container(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
        )
        .unwrap();

        assert!(result.starts_with("docker run --rm -it"));
        assert!(result.contains("--mount type=bind,source=/tmp/project,target=/tmp/project"));
        assert!(result.contains("--workdir /tmp/project"));
        assert!(result.contains("test-image:latest"));
        assert!(result.contains("sh -c 'claude'"));
    }

    #[test]
    fn test_wrap_escapes_quotes() {
        let config = make_config();
        let result = wrap_for_container(
            "echo 'hello'",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
        )
        .unwrap();

        assert!(result.contains("sh -c 'echo '\\''hello'\\'''"));
    }

    #[test]
    fn test_podman_runtime() {
        let config = SandboxConfig {
            enabled: Some(true),
            runtime: Some(SandboxRuntime::Podman),
            image: Some("test-image:latest".to_string()),
            ..Default::default()
        };
        let result = wrap_for_container(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
        )
        .unwrap();

        assert!(result.starts_with("podman run"));
    }

    #[test]
    fn test_wrap_with_subdir_cwd() {
        let config = make_config();
        let result = wrap_for_container(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project/backend"),
        )
        .unwrap();

        // Should mount the worktree root
        assert!(result.contains("--mount type=bind,source=/tmp/project,target=/tmp/project"));
        // But set workdir to the subdir
        assert!(result.contains("--workdir /tmp/project/backend"));
    }

    #[test]
    fn test_wrap_uses_default_image_when_not_configured() {
        let config = SandboxConfig {
            enabled: Some(true),
            image: None,
            ..Default::default()
        };
        let result = wrap_for_container(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
        )
        .unwrap();

        // Should use default image name
        assert!(result.contains("workmux-sandbox"));
    }
}
