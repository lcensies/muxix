//! Command wrapping for Lima backend.

use anyhow::{Context, Result};
use std::path::Path;

use super::{LimaInstance, generate_lima_config, generate_mounts, instance_name};
use crate::config::Config;

/// Escape a string for use in a single-quoted shell string.
fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// VM state detected before wrapping the command.
enum VmState {
    /// VM is already running, no boot needed
    Running,
    /// VM exists but is stopped, needs `limactl start <name>`
    Stopped,
    /// VM doesn't exist, needs `limactl start --name <name> <config>`
    NotFound,
}

/// Wrap a command to run inside a Lima VM.
///
/// Returns a self-contained shell command that ensures the VM is running and then
/// executes the original command via `limactl shell`. The VM boot happens inside
/// the tmux pane so the user sees progress output instead of an empty shell.
///
/// The VM existence check happens in Rust (fast, ~100ms) to determine the right
/// `limactl start` invocation, but the actual boot is deferred to the pane.
///
/// # Arguments
/// * `command` - The command to run (e.g., "claude", "bash")
/// * `config` - The workmux configuration
/// * `worktree_path` - Path to the worktree (used to determine project for isolation)
/// * `working_dir` - Working directory inside the VM
///
/// # Returns
/// A wrapped command that boots the VM (if needed) and runs the command inside it
pub fn wrap_for_lima(
    command: &str,
    config: &Config,
    worktree_path: &Path,
    working_dir: &Path,
) -> Result<String> {
    // Check if Lima is available (fast, no VM boot)
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

    // Write config to a stable temp path (needed for VM creation)
    let config_path = std::env::temp_dir().join(format!("workmux-lima-{}.yaml", vm_name));
    std::fs::write(&config_path, &lima_config)
        .with_context(|| format!("Failed to write Lima config to {}", config_path.display()))?;

    // Check VM state in Rust (fast ~100ms) to determine the right start command.
    // The actual boot is deferred to the pane so the user sees progress output.
    let vm_state = check_vm_state(&vm_name)?;

    // Build the start command prefix based on VM state
    let start_prefix = match vm_state {
        VmState::Running => {
            // VM already running, no boot needed
            String::new()
        }
        VmState::Stopped => {
            // VM exists but stopped, start it
            format!(
                "echo 'Starting Lima VM {}...' && limactl start --tty=false {} && ",
                vm_name, vm_name
            )
        }
        VmState::NotFound => {
            // VM doesn't exist, create and start it
            format!(
                "echo 'Creating Lima VM {}...' && limactl start --name {} --tty=false '{}' && ",
                vm_name,
                vm_name,
                shell_escape(&config_path.to_string_lossy())
            )
        }
    };

    // Build the limactl shell command
    let mut shell_cmd = format!("limactl shell {}", vm_name);

    // Pass through environment variables
    for env_var in config.sandbox.env_passthrough() {
        if let Ok(val) = std::env::var(env_var) {
            shell_cmd.push_str(&format!(" --setenv {}='{}'", env_var, shell_escape(&val)));
        }
    }

    // Build the inner script with properly quoted paths, then escape for sh -c.
    // The inner cd path needs its own quoting to handle spaces.
    let inner_script = format!(
        "cd '{}' && {}",
        shell_escape(&working_dir.to_string_lossy()),
        command
    );
    shell_cmd.push_str(&format!(" -- sh -c '{}'", shell_escape(&inner_script)));

    Ok(format!("{}{}", start_prefix, shell_cmd))
}

/// Check the current state of a Lima VM by name.
fn check_vm_state(vm_name: &str) -> Result<VmState> {
    let instances = LimaInstance::list()?;

    match instances.iter().find(|i| i.name == vm_name) {
        Some(info) if info.is_running() => Ok(VmState::Running),
        Some(_) => Ok(VmState::Stopped),
        None => Ok(VmState::NotFound),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::lima::LimaInstanceInfo;
    use std::path::PathBuf;

    #[test]
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello"), "hello");
        assert_eq!(shell_escape("foo bar"), "foo bar");
    }

    #[test]
    fn test_shell_escape_single_quotes() {
        assert_eq!(
            shell_escape("echo 'hello world'"),
            "echo '\\''hello world'\\''"
        );
    }

    #[test]
    fn test_shell_escape_preserves_special_chars() {
        // Single-quote escaping should not affect other shell metacharacters
        // (they're safe inside single quotes)
        assert_eq!(shell_escape("$HOME"), "$HOME");
        assert_eq!(shell_escape("$(cmd)"), "$(cmd)");
        assert_eq!(shell_escape("a & b"), "a & b");
    }

    #[test]
    fn test_shell_escape_path_with_spaces() {
        assert_eq!(
            shell_escape("/Users/test user/my project"),
            "/Users/test user/my project"
        );
    }

    #[test]
    fn test_check_vm_state_running() {
        // VmState enum should correctly categorize states
        let info = LimaInstanceInfo {
            name: "test-vm".to_string(),
            status: "Running".to_string(),
            dir: None,
        };
        assert!(info.is_running());
    }

    #[test]
    fn test_check_vm_state_stopped() {
        let info = LimaInstanceInfo {
            name: "test-vm".to_string(),
            status: "Stopped".to_string(),
            dir: None,
        };
        assert!(!info.is_running());
    }

    #[test]
    fn test_wrap_format_shell_command() {
        // Test the limactl shell command format (without calling wrap_for_lima
        // which requires limactl + git repo)
        let vm_name = "wm-abc12345";
        let working_dir = PathBuf::from("/Users/test/project");
        let command = "claude";

        let inner_script = format!(
            "cd '{}' && {}",
            shell_escape(&working_dir.to_string_lossy()),
            command
        );
        let shell_cmd = format!(
            "limactl shell {} -- sh -c '{}'",
            vm_name,
            shell_escape(&inner_script)
        );

        assert!(shell_cmd.contains("limactl shell wm-abc12345"));
        // Path without special chars: inner quotes get escaped for outer sh -c
        // sh will reconstruct: cd '/Users/test/project' && claude
        assert!(shell_cmd.contains("/Users/test/project"));
        assert!(shell_cmd.contains("claude"));
    }

    #[test]
    fn test_wrap_format_with_spaces_in_path() {
        let vm_name = "wm-abc12345";
        let working_dir = PathBuf::from("/Users/test user/my project");
        let command = "claude";

        let inner_script = format!(
            "cd '{}' && {}",
            shell_escape(&working_dir.to_string_lossy()),
            command
        );
        let shell_cmd = format!(
            "limactl shell {} -- sh -c '{}'",
            vm_name,
            shell_escape(&inner_script)
        );

        // The inner cd path is single-quoted, and those quotes get escaped
        // for the outer sh -c single-quote context. When sh parses the outer
        // quotes, the inner quotes are restored, giving: cd '/path with spaces'
        assert!(shell_cmd.contains("/Users/test user/my project"));
        // Verify the inner quotes are present (escaped as '\'' for outer context)
        assert!(shell_cmd.contains("cd '\\''"));
    }

    #[test]
    fn test_start_prefix_stopped_vm() {
        let vm_name = "wm-test123";
        let prefix = format!(
            "echo 'Starting Lima VM {}...' && limactl start --tty=false {} && ",
            vm_name, vm_name
        );
        assert!(prefix.contains("limactl start --tty=false wm-test123"));
        assert!(prefix.contains("echo"));
        assert!(prefix.ends_with("&& "));
    }

    #[test]
    fn test_start_prefix_new_vm() {
        let vm_name = "wm-test123";
        let config_path = "/tmp/workmux-lima-wm-test123.yaml";
        let prefix = format!(
            "echo 'Creating Lima VM {}...' && limactl start --name {} --tty=false '{}' && ",
            vm_name,
            vm_name,
            shell_escape(config_path)
        );
        assert!(prefix.contains("limactl start --name wm-test123 --tty=false"));
        assert!(prefix.contains(config_path));
        assert!(prefix.ends_with("&& "));
    }

    #[test]
    fn test_env_passthrough_escaping() {
        // Verify env var values with special characters are properly escaped
        let env_var = "MY_VAR";
        let val = "hello'world";
        let flag = format!(" --setenv {}='{}'", env_var, shell_escape(&val));
        assert_eq!(flag, " --setenv MY_VAR='hello'\\''world'");
    }
}
