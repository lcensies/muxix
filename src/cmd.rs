use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::process::{Command, Output};
use tracing::{debug, trace};

#[cfg(test)]
thread_local! {
    /// Git processes spawned on this thread. Lets a test assert how much work
    /// a code path does; thread-local so parallel tests cannot perturb it.
    static GIT_SPAWNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Count one git process spawn. Call this for a git child that is launched
/// directly rather than through [`Cmd`].
pub fn count_git_spawn() {
    #[cfg(test)]
    GIT_SPAWNS.with(|c| c.set(c.get() + 1));
}

/// Read and reset this thread's git spawn count.
#[cfg(test)]
pub fn take_git_spawns() -> usize {
    GIT_SPAWNS.with(|c| c.replace(0))
}

/// A builder for executing shell commands with unified error handling
pub struct Cmd<'a> {
    command: &'a str,
    args: Vec<&'a str>,
    workdir: Option<&'a Path>,
}

impl<'a> Cmd<'a> {
    /// Create a new command builder
    pub fn new(command: &'a str) -> Self {
        Self {
            command,
            args: Vec::new(),
            workdir: None,
        }
    }

    /// Add a single argument
    pub fn arg(mut self, arg: &'a str) -> Self {
        self.args.push(arg);
        self
    }

    /// Add multiple arguments
    pub fn args(mut self, args: &[&'a str]) -> Self {
        self.args.extend_from_slice(args);
        self
    }

    /// Set the working directory for the command
    pub fn workdir(mut self, path: &'a Path) -> Self {
        self.workdir = Some(path);
        self
    }

    /// Execute the command and return the output
    /// Returns an error if the command fails (non-zero exit code)
    pub fn run(self) -> Result<Output> {
        let Cmd {
            command,
            args,
            workdir,
        } = self;
        let workdir_display = workdir.map(|p| p.display().to_string());

        trace!(command, args = ?args, workdir = ?workdir_display, "cmd:run start");
        if command == "git" {
            count_git_spawn();
        }

        let mut cmd = Command::new(command);
        if let Some(dir) = workdir {
            cmd.current_dir(dir);
        }
        let output = cmd.args(&args).output().with_context(|| {
            format!("Failed to execute command: {} {}", command, args.join(" "))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            debug!(
                command,
                args = ?args,
                status = ?output.status.code(),
                stderr = %stderr.trim(),
                "cmd:run failure"
            );
            return Err(anyhow!(
                "Command failed: {} {}\n{}",
                command,
                args.join(" "),
                stderr.trim()
            ));
        }
        trace!(command, "cmd:run success");
        Ok(output)
    }

    /// Execute the command and return stdout as a trimmed string
    pub fn run_and_capture_stdout(self) -> Result<String> {
        let output = self.run()?;
        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    /// Execute the command, returning Ok(true) if it succeeds, Ok(false) if it fails
    /// This is useful for commands that are used as checks (e.g., git rev-parse --verify)
    pub fn run_as_check(self) -> Result<bool> {
        let Cmd {
            command,
            args,
            workdir,
        } = self;
        let workdir_display = workdir.map(|p| p.display().to_string());
        trace!(command, args = ?args, workdir = ?workdir_display, "cmd:check start");
        if command == "git" {
            count_git_spawn();
        }

        let mut cmd = Command::new(command);
        if let Some(dir) = workdir {
            cmd.current_dir(dir);
        }
        let output = cmd.args(&args).output().with_context(|| {
            format!("Failed to execute command: {} {}", command, args.join(" "))
        })?;

        let success = output.status.success();
        trace!(command, success, "cmd:check result");
        Ok(success)
    }
}

/// Helper to create a shell command with additional environment variables
pub fn shell_command_with_env(
    command: &str,
    workdir: &Path,
    env_vars: &[(&str, &str)],
) -> Result<()> {
    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(command).current_dir(workdir);

    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute shell command: {}", command))?;

    if !status.success() {
        return Err(anyhow!(
            "Shell command failed with exit code {}: {}",
            status.code().unwrap_or(-1),
            command
        ));
    }
    Ok(())
}
