//! Command wrapping for Lima backend.

use anyhow::Result;
use std::path::Path;

use crate::config::Config;

/// Escape a string for use in a single-quoted shell string.
fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

/// Wrap a command to run inside a Lima VM via `limactl shell`.
///
/// Returns the shell command that executes inside an already-running VM.
/// The VM must be booted before calling this (via `ensure_vm_running()`).
///
/// # Arguments
/// * `command` - The command to run (e.g., "claude", "bash")
/// * `config` - The workmux configuration (for env passthrough)
/// * `vm_name` - The Lima VM instance name (from `ensure_vm_running()`)
/// * `working_dir` - Working directory inside the VM
pub fn wrap_for_lima(
    command: &str,
    config: &Config,
    vm_name: &str,
    working_dir: &Path,
) -> Result<String> {
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

    Ok(shell_cmd)
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
        // LimaInstanceInfo correctly categorizes states
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
        // Test the limactl shell command format directly
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
    fn test_env_passthrough_escaping() {
        // Verify env var values with special characters are properly escaped
        let env_var = "MY_VAR";
        let val = "hello'world";
        let flag = format!(" --setenv {}='{}'", env_var, shell_escape(&val));
        assert_eq!(flag, " --setenv MY_VAR='hello'\\''world'");
    }
}
