//! Docker/Podman container sandbox implementation.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::config::{SandboxConfig, SandboxRuntime};
use crate::state::StateStore;

/// Default image registry prefix.
pub const DEFAULT_IMAGE_REGISTRY: &str = "ghcr.io/raine/workmux-sandbox";

/// Embedded Dockerfiles for each agent.
pub const DOCKERFILE_BASE: &str = include_str!("../../docker/Dockerfile.base");
pub const DOCKERFILE_CLAUDE: &str = include_str!("../../docker/Dockerfile.claude");
pub const DOCKERFILE_CODEX: &str = include_str!("../../docker/Dockerfile.codex");
pub const DOCKERFILE_GEMINI: &str = include_str!("../../docker/Dockerfile.gemini");
pub const DOCKERFILE_OPENCODE: &str = include_str!("../../docker/Dockerfile.opencode");

/// Known agents that have pre-built images.
pub const KNOWN_AGENTS: &[&str] = &["claude", "codex", "gemini", "opencode"];

/// Get the agent-specific Dockerfile content, or None for unknown agents.
pub fn dockerfile_for_agent(agent: &str) -> Option<&'static str> {
    match agent {
        "claude" => Some(DOCKERFILE_CLAUDE),
        "codex" => Some(DOCKERFILE_CODEX),
        "gemini" => Some(DOCKERFILE_GEMINI),
        "opencode" => Some(DOCKERFILE_OPENCODE),
        _ => None,
    }
}

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
pub fn run_auth(config: &SandboxConfig, agent: &str) -> Result<()> {
    let paths = ensure_sandbox_config_dirs()?;
    let runtime = match config.runtime() {
        SandboxRuntime::Podman => "podman",
        SandboxRuntime::Docker => "docker",
    };
    let image = config.resolved_image(agent);

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

/// Build the sandbox Docker image locally (two-stage: base + agent).
pub fn build_image(config: &SandboxConfig, agent: &str) -> Result<()> {
    let runtime = match config.runtime() {
        SandboxRuntime::Podman => "podman",
        SandboxRuntime::Docker => "docker",
    };

    let agent_dockerfile = dockerfile_for_agent(agent).ok_or_else(|| {
        anyhow::anyhow!(
            "No Dockerfile for agent '{}'. Known agents: {}",
            agent,
            KNOWN_AGENTS.join(", ")
        )
    })?;

    // Stage 1: Build base image (use localhost/ prefix for Podman compatibility)
    let base_tag = "localhost/workmux-sandbox-base";
    println!("Building base image...");

    let tmp_dir = tempfile::tempdir().context("Failed to create temp dir")?;
    std::fs::write(tmp_dir.path().join("Dockerfile"), DOCKERFILE_BASE)?;

    let status = Command::new(runtime)
        .env("DOCKER_BUILDKIT", "1")
        .args(["build", "-t", base_tag, "-f", "Dockerfile", "."])
        .current_dir(tmp_dir.path())
        .status()
        .context("Failed to build base image")?;

    if !status.success() {
        anyhow::bail!("Failed to build base image");
    }

    // Stage 2: Build agent image on top of local base
    let image = config.resolved_image(agent);
    println!("Building {} image...", agent);

    let agent_tmp = tempfile::tempdir().context("Failed to create temp dir")?;
    std::fs::write(agent_tmp.path().join("Dockerfile"), agent_dockerfile)?;

    let status = Command::new(runtime)
        .env("DOCKER_BUILDKIT", "1")
        .args([
            "build",
            "--build-arg",
            &format!("BASE={}", base_tag),
            "-t",
            &image,
            "-f",
            "Dockerfile",
            ".",
        ])
        .current_dir(agent_tmp.path())
        .status()
        .context("Failed to build agent image")?;

    if !status.success() {
        anyhow::bail!("Failed to build image '{}'", image);
    }

    Ok(())
}

/// Pull the sandbox image from the registry.
pub fn pull_image(config: &SandboxConfig, image: &str) -> Result<()> {
    let runtime = match config.runtime() {
        SandboxRuntime::Podman => "podman",
        SandboxRuntime::Docker => "docker",
    };

    println!("Pulling image '{}'...", image);

    let status = Command::new(runtime)
        .args(["pull", image])
        .status()
        .context("Failed to run container runtime")?;

    if !status.success() {
        anyhow::bail!("Failed to pull image '{}'", image);
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
    agent: &str,
    worktree_root: &Path,
    pane_cwd: &Path,
    extra_envs: &[(&str, &str)],
    shim_host_dir: Option<&Path>,
) -> Result<Vec<String>> {
    let image = config.resolved_image(agent);
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

    // Git worktree mounts: .git directory + main worktree (for symlink resolution)
    let git_path = worktree_root.join(".git");
    if git_path.is_file()
        && let Ok(content) = std::fs::read_to_string(&git_path)
        && let Some(gitdir) = content.strip_prefix("gitdir: ")
    {
        let gitdir = gitdir.trim();
        if let Some(main_git) = Path::new(gitdir).ancestors().nth(2) {
            // Mount the .git directory for git operations
            args.push("--mount".to_string());
            args.push(format!(
                "type=bind,source={},target={}",
                main_git.display(),
                main_git.display()
            ));

            // Mount the main worktree to resolve symlinks pointing there
            // (e.g., CLAUDE.md -> ../../main-worktree/CLAUDE.md)
            if let Some(main_worktree) = main_git.parent() {
                args.push("--mount".to_string());
                args.push(format!(
                    "type=bind,source={},target={}",
                    main_worktree.display(),
                    main_worktree.display()
                ));
            }
        }
    }

    // Bind-mount shim directory if host-exec is configured
    if let Some(shim_dir) = shim_host_dir {
        args.push("--mount".to_string());
        args.push(format!(
            "type=bind,source={},target=/tmp/.workmux-shims/bin,readonly",
            shim_dir.display()
        ));
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
    // Prepend shim directory when host-exec is configured
    let path = if shim_host_dir.is_some() {
        "/tmp/.workmux-shims/bin:/tmp/.local/bin:/root/.local/bin:/usr/local/bin:/usr/bin:/bin"
    } else {
        "/tmp/.local/bin:/root/.local/bin:/usr/local/bin:/usr/bin:/bin"
    };
    args.push("--env".to_string());
    args.push(format!("PATH={}", path));

    // Image
    args.push(image.to_string());

    // Command: sh -c <command>
    // No shell quoting needed -- callers use Command::args() which handles escaping
    args.push("sh".to_string());
    args.push("-c".to_string());
    args.push(command.to_string());

    Ok(args)
}

use crate::shell::shell_escape;

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

/// Stop any running containers associated with a worktree handle.
///
/// Uses the state store to find registered containers instead of running
/// `docker ps`. This avoids spawning docker commands for users who don't
/// use containers.
pub fn stop_containers_for_handle(handle: &str, config: &SandboxConfig) {
    // Check state store for registered containers
    let store = match StateStore::new() {
        Ok(s) => s,
        Err(_) => return,
    };

    let containers = store.list_containers(handle);
    if containers.is_empty() {
        return;
    }

    let runtime = match config.runtime() {
        SandboxRuntime::Podman => "podman",
        SandboxRuntime::Docker => "docker",
    };

    tracing::debug!(?containers, handle, "stopping containers for worktree");

    // Stop all containers in one command
    let _ = Command::new(runtime)
        .arg("stop")
        .arg("-t")
        .arg("2")
        .args(&containers)
        .output();

    // Unregister containers from state store
    for name in containers {
        store.unregister_container(handle, &name);
    }
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
            "claude",
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[],
            None,
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
            "claude",
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[("WM_SANDBOX_GUEST", "1"), ("WM_RPC_PORT", "12345")],
            None,
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
            "claude",
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[],
            None,
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
            "claude",
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[],
            None,
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
            "claude",
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[],
            None,
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

    #[test]
    fn test_build_args_with_shims() {
        let config = make_config();
        let tmp = tempfile::tempdir().unwrap();
        let shim_bin = tmp.path().join("shims/bin");
        std::fs::create_dir_all(&shim_bin).unwrap();

        let args = build_docker_run_args(
            "claude",
            &config,
            "claude",
            Path::new("/tmp/project"),
            Path::new("/tmp/project"),
            &[],
            Some(&shim_bin),
        )
        .unwrap();

        let args_str = args.join(" ");
        // Shim dir should be bind-mounted
        assert!(args_str.contains(".workmux-shims/bin"));
        // PATH should include shim dir first
        let path_arg = args.iter().find(|a| a.starts_with("PATH=")).unwrap();
        assert!(path_arg.starts_with("PATH=/tmp/.workmux-shims/bin:"));
    }

    #[test]
    fn test_dockerfile_for_known_agents() {
        assert!(dockerfile_for_agent("claude").is_some());
        assert!(dockerfile_for_agent("codex").is_some());
        assert!(dockerfile_for_agent("gemini").is_some());
        assert!(dockerfile_for_agent("opencode").is_some());
    }

    #[test]
    fn test_dockerfile_for_unknown_agent() {
        assert!(dockerfile_for_agent("unknown").is_none());
        assert!(dockerfile_for_agent("default").is_none());
    }

    #[test]
    fn test_default_image_resolution() {
        let config = SandboxConfig::default();
        assert_eq!(
            config.resolved_image("claude"),
            "ghcr.io/raine/workmux-sandbox:claude"
        );
        assert_eq!(
            config.resolved_image("codex"),
            "ghcr.io/raine/workmux-sandbox:codex"
        );
    }

    #[test]
    fn test_custom_image_resolution() {
        let config = SandboxConfig {
            image: Some("my-image:latest".to_string()),
            ..Default::default()
        };
        assert_eq!(config.resolved_image("claude"), "my-image:latest");
    }
}
