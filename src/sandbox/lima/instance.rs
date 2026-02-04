//! Lima VM instance management.

use anyhow::{Context, Result, bail};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

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

/// Run a limactl command with comprehensive logging and timeout.
/// Returns the command output if successful, or an error if it fails or times out.
fn run_limactl(args: &[&str], timeout: Duration) -> Result<Output> {
    // Log the command before execution
    tracing::debug!("executing limactl command: limactl {}", args.join(" "));

    // Spawn the command in a thread to enable timeout handling
    let args_vec: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    let handle = std::thread::spawn(move || Command::new("limactl").args(&args_vec).output());

    // Wait for the thread with timeout
    let start = std::time::Instant::now();
    loop {
        if start.elapsed() >= timeout {
            // Timeout occurred - the thread will be detached and the process may continue running
            tracing::warn!(
                "limactl command timed out after {} seconds, command may still be running in background",
                timeout.as_secs()
            );
            bail!(
                "limactl command timed out after {} seconds: limactl {}",
                timeout.as_secs(),
                args.join(" ")
            );
        }

        if handle.is_finished() {
            let output = handle
                .join()
                .map_err(|_| anyhow::anyhow!("thread panicked"))?
                .context("failed to execute limactl command")?;

            // Log stdout if not empty
            if !output.stdout.is_empty() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                tracing::debug!("limactl stdout: {}", stdout.trim());
            }

            // Log stderr if not empty
            if !output.stderr.is_empty() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if output.status.success() {
                    tracing::debug!("limactl stderr: {}", stderr.trim());
                } else {
                    tracing::info!("limactl stderr: {}", stderr.trim());
                }
            }

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("limactl command failed: {}", stderr);
            }

            return Ok(output);
        }

        std::thread::sleep(Duration::from_millis(100));
    }
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
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        spinner.set_message(format!("Starting existing VM {}...", self.name));
        spinner.enable_steady_tick(Duration::from_millis(100));

        let start_time = Instant::now();
        let timeout = Duration::from_secs(300); // 5 minutes for VM creation
        let result = run_limactl(&["start", "--tty=false", &self.name], timeout);
        let elapsed = start_time.elapsed();

        match result {
            Ok(_) => {
                spinner.finish_with_message(format!(
                    "Started existing VM {} ({:.1}s)",
                    self.name,
                    elapsed.as_secs_f64()
                ));
            }
            Err(e) => {
                spinner.finish_and_clear();
                return Err(e).with_context(|| format!("failed to start Lima VM '{}'", self.name));
            }
        }
        Ok(())
    }

    /// Create and start a new Lima VM instance using the config file.
    fn create_and_start(&self) -> Result<()> {
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        spinner.set_message(format!(
            "Starting Lima VM {} (first boot takes ~30s)...",
            self.name
        ));
        spinner.enable_steady_tick(Duration::from_millis(100));

        let start_time = Instant::now();
        let timeout = Duration::from_secs(300); // 5 minutes for VM creation
        let config_path_str = self.config_path.to_string_lossy();
        let result = run_limactl(
            &[
                "start",
                "--name",
                &self.name,
                "--tty=false",
                &config_path_str,
            ],
            timeout,
        );
        let elapsed = start_time.elapsed();

        match result {
            Ok(_) => {
                spinner.finish_with_message(format!(
                    "Started Lima VM {} ({:.1}s)",
                    self.name,
                    elapsed.as_secs_f64()
                ));
            }
            Err(e) => {
                spinner.finish_and_clear();
                return Err(e).with_context(|| format!("failed to create Lima VM '{}'", self.name));
            }
        }
        Ok(())
    }

    /// Stop the Lima VM.
    #[allow(dead_code)]
    pub fn stop(&self) -> Result<()> {
        let timeout = Duration::from_secs(30);
        run_limactl(&["stop", &self.name], timeout)
            .with_context(|| format!("failed to stop Lima VM '{}'", self.name))?;
        Ok(())
    }

    /// Check if the Lima VM is running.
    pub fn is_running(&self) -> Result<bool> {
        let timeout = Duration::from_secs(10);
        let output =
            run_limactl(&["list", "--json"], timeout).context("failed to list Lima instances")?;

        let instances = parse_lima_instances(&output.stdout)?;

        Ok(instances
            .iter()
            .any(|i| i.name == self.name && i.status == "Running"))
    }

    /// Execute a shell command in the Lima VM.
    #[allow(dead_code)]
    pub fn shell(&self, command: &str) -> Result<String> {
        let timeout = Duration::from_secs(60);
        let output = run_limactl(&["shell", &self.name, "--", "sh", "-c", command], timeout)
            .with_context(|| format!("failed to execute command in VM '{}'", self.name))?;

        Ok(String::from_utf8(output.stdout)?)
    }

    /// Get the instance name.
    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Check if limactl is available on the system.
    pub fn is_lima_available() -> bool {
        tracing::debug!("checking if limactl is available");
        let timeout = Duration::from_secs(5);
        match run_limactl(&["--version"], timeout) {
            Ok(_) => {
                tracing::debug!("limactl is available");
                true
            }
            Err(e) => {
                tracing::debug!("limactl is not available: {}", e);
                false
            }
        }
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
        let timeout = Duration::from_secs(10);
        let output =
            run_limactl(&["list", "--json"], timeout).context("failed to list Lima instances")?;

        let instances = parse_lima_instances(&output.stdout)?;

        let exists = instances.iter().any(|i| i.name == name);
        if exists {
            // Start existing instance (without config file)
            instance.start()?;
            return Ok(instance);
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
