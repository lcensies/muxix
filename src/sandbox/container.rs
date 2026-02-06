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

# Install Claude Code and make it accessible by all users
# (container runs as host UID, not root, with HOME=/tmp)
RUN curl -fsSL https://claude.ai/install.sh | bash && \
    chmod a+x /root && \
    chmod -R a+rX /root/.local /root/.claude && \
    mkdir -p /tmp/.local && \
    ln -s /root/.local/bin /tmp/.local/bin

# Copy workmux binary from build context
COPY workmux /usr/local/bin/workmux
RUN chmod +x /usr/local/bin/workmux

# Install afplay shim that routes sound playback to host via RPC
RUN printf '#!/bin/sh\nexec workmux notify sound "$@"\n' > /usr/local/bin/afplay && \
    chmod +x /usr/local/bin/afplay

# Add claude to PATH (installed to .local/bin by installer)
ENV PATH="/root/.local/bin:${PATH}"
"#;

/// Sandbox-specific config paths on host.
/// The config file (~/.claude-sandbox.json) is separate from host CLI config
/// to avoid confusion, while ~/.claude/ is shared from the host.
pub struct SandboxPaths {
    /// ~/.claude-sandbox.json - main config/auth file
    pub config_file: PathBuf,
}

impl SandboxPaths {
    pub fn new() -> Option<Self> {
        let home = home::home_dir()?;
        Some(Self {
            config_file: home.join(".claude-sandbox.json"),
        })
    }
}

/// Ensure sandbox config files exist on host.
pub fn ensure_sandbox_config_dirs() -> Result<SandboxPaths> {
    let paths = SandboxPaths::new().context("Could not determine home directory")?;

    // Create empty config file if it doesn't exist
    if !paths.config_file.exists() {
        std::fs::write(&paths.config_file, "{}")
            .with_context(|| format!("Failed to create {}", paths.config_file.display()))?;
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

    let mut args = vec![
        "run".to_string(),
        "-it".to_string(),
        "--rm".to_string(),
        // Mount sandbox-specific config (read-write for auth)
        "--mount".to_string(),
        format!(
            "type=bind,source={},target=/tmp/.claude.json",
            paths.config_file.display()
        ),
    ];

    // Mount host ~/.claude/ directory so credentials and settings are available
    if let Some(home) = home::home_dir() {
        let claude_dir = home.join(".claude");
        if claude_dir.exists() {
            args.push("--mount".to_string());
            args.push(format!(
                "type=bind,source={},target=/tmp/.claude",
                claude_dir.display()
            ));
        }
    }

    args.extend([
        // Set HOME to /tmp where config is mounted
        "--env".to_string(),
        "HOME=/tmp".to_string(),
        // PATH: include both /root/.local/bin (where Claude is installed) and
        // /tmp/.local/bin (symlink, so Claude sees $HOME/.local/bin in PATH)
        "--env".to_string(),
        "PATH=/tmp/.local/bin:/root/.local/bin:/usr/local/bin:/usr/bin:/bin".to_string(),
        image.to_string(),
        "claude".to_string(),
    ]);

    let status = Command::new(runtime)
        .args(&args)
        .status()
        .context("Failed to run container")?;

    if !status.success() {
        anyhow::bail!("Auth container exited with status: {}", status);
    }

    Ok(())
}

/// Build the sandbox container image.
///
/// Creates a minimal build context with the provided workmux binary and an
/// embedded Dockerfile, then runs docker/podman build.
///
/// # Arguments
/// * `config` - Sandbox configuration
/// * `workmux_binary` - Path to a Linux workmux binary to include in the image
pub fn build_image(config: &SandboxConfig, workmux_binary: &Path) -> Result<()> {
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

    // Copy workmux binary to build context
    let dest_exe = context_path.join("workmux");
    std::fs::copy(workmux_binary, &dest_exe)
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

/// Build the argument list for a `docker run` command.
///
/// Returns the full arg vector (excluding the runtime binary name itself).
/// Used by the sandbox supervisor to run containers with RPC connection details.
///
/// Callers must:
/// - Prepend the runtime binary name (docker/podman)
/// - Call `ensure_sandbox_config_dirs()` before this function if config mounts are needed
/// - Use `Command::args()` (not string joining) since args are not shell-quoted
pub fn build_docker_run_args(
    command: &str,
    config: &SandboxConfig,
    worktree_root: &Path,
    pane_cwd: &Path,
    extra_envs: &[(&str, &str)],
) -> Result<Vec<String>> {
    let image = config.resolved_image();
    let worktree_root_str = worktree_root.to_string_lossy();
    let pane_cwd_str = pane_cwd.to_string_lossy();

    let uid = unsafe { libc::getuid() };
    let gid = unsafe { libc::getgid() };

    let mut args = Vec::new();

    // Base command (no runtime name -- caller prepends that)
    args.push("run".to_string());
    args.push("--rm".to_string());
    args.push("-it".to_string());

    // On Linux Docker Engine (not Desktop), host.docker.internal doesn't resolve
    // unless we explicitly add it. The special "host-gateway" value maps to the
    // host's gateway IP. This is a harmless no-op on Docker Desktop.
    if matches!(config.runtime(), SandboxRuntime::Docker) {
        args.push("--add-host".to_string());
        args.push("host.docker.internal:host-gateway".to_string());
    }

    args.push("--user".to_string());
    args.push(format!("{}:{}", uid, gid));

    // Mirror mount worktree
    args.push("--mount".to_string());
    args.push(format!(
        "type=bind,source={},target={}",
        worktree_root_str, worktree_root_str
    ));

    // Git worktree .git directory mount
    let git_path = worktree_root.join(".git");
    if git_path.is_file()
        && let Ok(content) = std::fs::read_to_string(&git_path)
        && let Some(gitdir) = content.strip_prefix("gitdir: ")
    {
        let gitdir = gitdir.trim();
        if let Some(main_git) = Path::new(gitdir).ancestors().nth(2) {
            args.push("--mount".to_string());
            args.push(format!(
                "type=bind,source={},target={}",
                main_git.display(),
                main_git.display()
            ));
        }
    }

    args.push("--workdir".to_string());
    args.push(pane_cwd_str.to_string());

    args.push("--env".to_string());
    args.push("HOME=/tmp".to_string());

    // Config mounts (caller must ensure dirs exist via ensure_sandbox_config_dirs)
    if let Some(paths) = SandboxPaths::new()
        && paths.config_file.exists()
    {
        args.push("--mount".to_string());
        args.push(format!(
            "type=bind,source={},target=/tmp/.claude.json",
            paths.config_file.display()
        ));
    }

    if let Some(home) = home::home_dir() {
        let claude_dir = home.join(".claude");
        if claude_dir.exists() {
            args.push("--mount".to_string());
            args.push(format!(
                "type=bind,source={},target=/tmp/.claude",
                claude_dir.display()
            ));
        }
    }

    // Terminal vars
    for term_var in ["TERM", "COLORTERM"] {
        if std::env::var(term_var).is_ok() {
            args.push("--env".to_string());
            args.push(term_var.to_string());
        }
    }

    // Env passthrough
    for var in config.env_passthrough() {
        if std::env::var(var).is_ok() {
            args.push("--env".to_string());
            args.push(var.to_string());
        }
    }

    // Extra env vars (RPC connection details)
    for (key, value) in extra_envs {
        args.push("--env".to_string());
        args.push(format!("{}={}", key, value));
    }

    // PATH: include both /root/.local/bin (where Claude is installed) and
    // /tmp/.local/bin (symlink, but needed so Claude sees $HOME/.local/bin in PATH)
    args.push("--env".to_string());
    args.push("PATH=/tmp/.local/bin:/root/.local/bin:/usr/local/bin:/usr/bin:/bin".to_string());

    // Image
    args.push(image.to_string());

    // Command: sh -c <command>
    // No shell quoting needed -- callers use Command::args() which handles escaping
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(command.to_string());

    Ok(args)
}

/// Escape a string for use in a single-quoted shell string.
fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Wrap a command to run inside a Docker/Podman container via the sandbox supervisor.
///
/// Generates a `workmux sandbox run` command that starts an RPC server, then
/// runs the command inside a container with RPC connection details as env vars.
pub fn wrap_for_container(
    command: &str,
    _config: &SandboxConfig,
    worktree_root: &Path,
    pane_cwd: &Path,
) -> Result<String> {
    // Strip the single leading space that rewrite_agent_command adds for
    // shell history prevention -- not needed for the supervisor.
    let command = command.strip_prefix(' ').unwrap_or(command);

    let mut parts = format!(
        "workmux sandbox run '{}'",
        shell_escape(&pane_cwd.to_string_lossy()),
    );

    // Only add --worktree-root when it differs from pane_cwd
    if worktree_root != pane_cwd {
        parts.push_str(&format!(
            " --worktree-root '{}'",
            shell_escape(&worktree_root.to_string_lossy()),
        ));
    }

    parts.push_str(&format!(" -- '{}'", shell_escape(command)));

    Ok(parts)
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
    fn test_build_args_basic() {
        let config = make_config();
        let args = build_docker_run_args(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[],
        )
        .unwrap();

        assert!(args.contains(&"run".to_string()));
        assert!(args.contains(&"--rm".to_string()));
        assert!(args.contains(&"-it".to_string()));
        assert!(args.contains(&"test-image:latest".to_string()));
        assert!(args.contains(&"sh".to_string()));
        assert!(args.contains(&"-c".to_string()));
        assert!(args.contains(&"claude".to_string()));
    }

    #[test]
    fn test_build_args_extra_envs() {
        let config = make_config();
        let args = build_docker_run_args(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[("WM_SANDBOX_GUEST", "1"), ("WM_RPC_PORT", "12345")],
        )
        .unwrap();

        assert!(args.contains(&"WM_SANDBOX_GUEST=1".to_string()));
        assert!(args.contains(&"WM_RPC_PORT=12345".to_string()));
    }

    #[test]
    fn test_build_args_docker_includes_add_host() {
        let config = SandboxConfig {
            enabled: Some(true),
            runtime: Some(SandboxRuntime::Docker),
            image: Some("test-image:latest".to_string()),
            ..Default::default()
        };
        let args = build_docker_run_args(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[],
        )
        .unwrap();

        assert!(args.contains(&"--add-host".to_string()));
        assert!(args.contains(&"host.docker.internal:host-gateway".to_string()));
    }

    #[test]
    fn test_build_args_podman_omits_add_host() {
        let config = SandboxConfig {
            enabled: Some(true),
            runtime: Some(SandboxRuntime::Podman),
            image: Some("test-image:latest".to_string()),
            ..Default::default()
        };
        let args = build_docker_run_args(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[],
        )
        .unwrap();

        assert!(!args.contains(&"--add-host".to_string()));
    }

    #[test]
    fn test_build_args_runtime_not_in_args() {
        let config = SandboxConfig {
            enabled: Some(true),
            runtime: Some(SandboxRuntime::Podman),
            image: Some("test-image:latest".to_string()),
            ..Default::default()
        };
        let args = build_docker_run_args(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[],
        )
        .unwrap();

        assert!(!args.contains(&"podman".to_string()));
        assert!(!args.contains(&"docker".to_string()));
    }

    #[test]
    fn test_wrap_generates_supervisor_command() {
        let config = make_config();
        let result = wrap_for_container(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
        )
        .unwrap();

        assert!(result.starts_with("workmux sandbox run"));
        assert!(result.contains("'/tmp/project'"));
        assert!(result.contains("-- 'claude'"));
        // Should NOT contain --worktree-root when paths are equal
        assert!(!result.contains("--worktree-root"));
    }

    #[test]
    fn test_wrap_escapes_quotes_in_command() {
        let config = make_config();
        let result = wrap_for_container(
            "echo 'hello'",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
        )
        .unwrap();

        assert!(result.contains("echo '\\''hello'\\''"));
    }

    #[test]
    fn test_wrap_strips_leading_space() {
        let config = make_config();
        let result = wrap_for_container(
            " claude -- \"$(cat PROMPT.md)\"",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
        )
        .unwrap();

        assert!(result.contains("-- 'claude -- \"$(cat PROMPT.md)\"'"));
    }

    #[test]
    fn test_wrap_with_different_worktree_root() {
        let config = make_config();
        let result = wrap_for_container(
            "claude",
            &config,
            Path::new("/tmp/project"),
            Path::new("/tmp/project/backend"),
        )
        .unwrap();

        assert!(result.contains("--worktree-root '/tmp/project'"));
        assert!(result.contains("'/tmp/project/backend'"));
    }
}
