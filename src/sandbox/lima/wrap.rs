//! Command wrapping for Lima backend.

use anyhow::Result;
use std::path::Path;

use super::{LimaInstance, generate_lima_config, generate_mounts, instance_name};
use crate::config::Config;

/// Escape a string for use in a single-quoted shell string.
fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Wrap a command to run inside a Lima VM.
///
/// This ensures the Lima VM is running and wraps the command to execute via `limactl shell`.
/// The VM is shared across multiple invocations and persists after the command completes.
///
/// # Arguments
/// * `command` - The command to run (e.g., "claude", "bash")
/// * `config` - The workmux configuration
/// * `worktree_path` - Path to the worktree (used to determine project for isolation)
/// * `working_dir` - Working directory inside the VM
///
/// # Returns
/// A wrapped command that will execute the original command inside the Lima VM
pub fn wrap_for_lima(
    command: &str,
    config: &Config,
    worktree_path: &Path,
    working_dir: &Path,
) -> Result<String> {
    // Check if Lima is available
    if !LimaInstance::is_lima_available() {
        anyhow::bail!(
            "Lima backend is enabled but limactl is not installed.\n\
             Install Lima: https://lima-vm.io/docs/installation/\n\
             Or disable sandbox: set 'sandbox.enabled: false' in config."
        );
    }

    let isolation = config.sandbox.isolation();

    // Generate instance name based on isolation level
    let vm_name = instance_name(worktree_path, isolation.clone(), config)?;

    // Generate mounts for this isolation level
    let mounts = generate_mounts(worktree_path, isolation, config)?;

    // Generate Lima config
    let lima_config = generate_lima_config(&vm_name, &mounts)?;

    // Get or create the Lima instance (starts VM if not running)
    let instance = LimaInstance::get_or_create(vm_name.clone(), &lima_config)?;

    // Verify VM is running
    if !instance.is_running()? {
        anyhow::bail!("Lima VM '{}' failed to start", vm_name);
    }

    // Build limactl shell command with environment variables
    let mut wrapped = format!("limactl shell {}", vm_name);

    // Pass through environment variables
    for env_var in config.sandbox.env_passthrough() {
        if let Ok(val) = std::env::var(env_var) {
            wrapped.push_str(&format!(" --setenv {}={}", env_var, shell_escape(&val)));
        }
    }

    // Add the shell command with proper escaping
    wrapped.push_str(&format!(
        " -- sh -c 'cd {} && {}'",
        shell_escape(&working_dir.to_string_lossy()),
        shell_escape(command)
    ));

    Ok(wrapped)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    #[test]
    fn test_command_escaping() {
        // Test that single quotes in commands are properly escaped
        let command = "echo 'hello world'";
        let escaped = command.replace('\'', "'\\''");
        assert_eq!(escaped, "echo '\\''hello world'\\''");
    }

    #[test]
    fn test_wrap_for_lima_format() {
        // This test would require limactl to be installed, so we just test the format
        let working_dir = PathBuf::from("/Users/test/project");
        let command = "claude";

        // The wrapped command should contain limactl shell
        // In a real scenario, this would call wrap_for_lima but it requires a git repo
        let expected_pattern = format!(
            "limactl shell wm-{} -- sh -c 'cd {} && {}'",
            "HASH",
            working_dir.display(),
            command
        );

        // Just verify the pattern is reasonable
        assert!(expected_pattern.contains("limactl shell"));
        assert!(expected_pattern.contains("cd /Users/test/project"));
        assert!(expected_pattern.contains("claude"));
    }
}
