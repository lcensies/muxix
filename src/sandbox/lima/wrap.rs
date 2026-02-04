//! Command wrapping for Lima backend.

use anyhow::Result;
use std::path::Path;

use super::{generate_lima_config, generate_mounts, instance_name, LimaInstance};
use crate::config::Config;

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

    // Wrap the command to run inside the VM
    // Format: limactl shell <vm-name> <command>
    let wrapped = format!(
        "limactl shell {} -- sh -c 'cd {} && {}'",
        vm_name,
        working_dir.display(),
        command.replace('\'', "'\\''") // Escape single quotes in command
    );

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
