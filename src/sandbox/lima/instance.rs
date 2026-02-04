//! Lima VM instance management.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Lima instance information from `limactl list --json`.
#[derive(Debug, Deserialize, Serialize)]
pub struct LimaInstanceInfo {
    pub name: String,
    pub status: String,
    #[serde(default)]
    pub dir: Option<String>,
}

/// Parse NDJSON output from `limactl list --json` (one JSON object per line).
pub fn parse_lima_instances(stdout: &[u8]) -> Result<Vec<LimaInstanceInfo>> {
    std::str::from_utf8(stdout)?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str::<LimaInstanceInfo>(l)
                .with_context(|| format!("Failed to parse limactl row: {}", l))
        })
        .collect()
}

/// Execute a command and stream its output in real-time using byte-streaming.
/// This preserves progress bars with carriage returns and handles invalid UTF-8.
/// Returns an error if the command fails with a non-zero exit code.
fn stream_command(mut command: Command) -> Result<()> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn command")?;

    let stdout = child.stdout.take().context("Failed to capture stdout")?;
    let stderr = child.stderr.take().context("Failed to capture stderr")?;

    // Spawn thread to stream stdout
    let stdout_handle = std::thread::spawn(move || -> std::io::Result<()> {
        let mut reader = stdout;
        let mut writer = std::io::stdout().lock();
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n])?;
            writer.flush()?; // Ensures real-time output even when stdout is piped
        }
        Ok(())
    });

    // Spawn thread to stream stderr
    let stderr_handle = std::thread::spawn(move || -> std::io::Result<()> {
        let mut reader = stderr;
        let mut writer = std::io::stderr().lock();
        let mut buf = [0u8; 8192];
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            writer.write_all(&buf[..n])?;
            writer.flush()?; // Ensures real-time output even when stderr is piped
        }
        Ok(())
    });

    // Wait for the command to finish
    let status = child.wait().context("Failed to wait for command")?;

    // Wait for output threads and propagate any errors
    stdout_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stdout thread panicked"))??;
    stderr_handle
        .join()
        .map_err(|_| anyhow::anyhow!("stderr thread panicked"))??;

    if !status.success() {
        bail!("Command failed with exit code: {:?}", status.code());
    }

    Ok(())
}

/// A Lima VM instance.
pub struct LimaInstance {
    name: String,
    config_path: PathBuf,
}

impl LimaInstance {
    /// Create a new Lima instance with the given name and config.
    /// The config YAML string will be written to a temp file.
    pub fn create(name: String, config: &str) -> Result<Self> {
        // Write config to temp file
        let config_path = std::env::temp_dir().join(format!("workmux-lima-{}.yaml", name));
        std::fs::write(&config_path, config)
            .with_context(|| format!("Failed to write Lima config to {}", config_path.display()))?;

        Ok(Self { name, config_path })
    }

    /// Start an existing Lima VM (without config file).
    pub fn start(&self) -> Result<()> {
        let mut command = Command::new("limactl");
        command.arg("start").arg("--tty=false").arg(&self.name);

        stream_command(command)
            .with_context(|| format!("Failed to start Lima VM '{}'", self.name))?;

        Ok(())
    }

    /// Create and start a new Lima VM instance using the config file.
    fn create_and_start(&self) -> Result<()> {
        let mut command = Command::new("limactl");
        command
            .arg("start")
            .arg("--name")
            .arg(&self.name)
            .arg("--tty=false")
            .arg(&self.config_path);

        stream_command(command)
            .with_context(|| format!("Failed to create Lima VM '{}'", self.name))?;

        Ok(())
    }

    /// Stop the Lima VM.
    #[allow(dead_code)]
    pub fn stop(&self) -> Result<()> {
        let mut command = Command::new("limactl");
        command.arg("stop").arg(&self.name);

        stream_command(command)
            .with_context(|| format!("Failed to stop Lima VM '{}'", self.name))?;

        Ok(())
    }

    /// Check if the Lima VM is running.
    pub fn is_running(&self) -> Result<bool> {
        let output = Command::new("limactl")
            .arg("list")
            .arg("--json")
            .output()
            .context("Failed to execute limactl list")?;

        if !output.status.success() {
            bail!("Failed to list Lima instances");
        }

        let instances = parse_lima_instances(&output.stdout)?;

        Ok(instances
            .iter()
            .any(|i| i.name == self.name && i.status == "Running"))
    }

    /// Execute a shell command in the Lima VM.
    #[allow(dead_code)]
    pub fn shell(&self, command: &str) -> Result<String> {
        let output = Command::new("limactl")
            .arg("shell")
            .arg(&self.name)
            .arg("--")
            .arg("sh")
            .arg("-c")
            .arg(command)
            .output()
            .context("Failed to execute limactl shell")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Command failed in VM '{}': {}", self.name, stderr);
        }

        Ok(String::from_utf8(output.stdout)?)
    }

    /// Get the instance name.
    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Check if limactl is available on the system.
    pub fn is_lima_available() -> bool {
        Command::new("limactl")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Get or create a Lima instance with the given name and config.
    /// If the instance already exists and is running, returns it without recreating.
    /// If it exists but is stopped, starts it.
    /// If it doesn't exist, creates and starts it.
    pub fn get_or_create(name: String, config: &str) -> Result<Self> {
        let instance = Self::create(name.clone(), config)?;

        // Check if already running
        if instance.is_running()? {
            return Ok(instance);
        }

        // Check if exists but stopped
        let output = Command::new("limactl")
            .arg("list")
            .arg("--json")
            .output()
            .context("Failed to execute limactl list")?;

        if output.status.success() {
            let instances = parse_lima_instances(&output.stdout)?;

            let exists = instances.iter().any(|i| i.name == name);
            if exists {
                // Start existing instance (without config file)
                instance.start()?;
                return Ok(instance);
            }
        }

        // Create and start new instance (with config file)
        instance.create_and_start()?;
        Ok(instance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instance_creation() {
        let instance =
            LimaInstance::create("test-vm".to_string(), "# Test config\nimages: []\n").unwrap();

        assert_eq!(instance.name(), "test-vm");
        assert!(instance.config_path.exists());
    }
}
