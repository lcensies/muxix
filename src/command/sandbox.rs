//! Sandbox management commands.

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use crate::config::Config;
use crate::sandbox;
use crate::sandbox::lima::{LimaInstance, parse_lima_instances};

#[derive(Debug, Args)]
pub struct SandboxArgs {
    #[command(subcommand)]
    pub command: SandboxCommand,
}

#[derive(Debug, Subcommand)]
pub enum SandboxCommand {
    /// Authenticate with the agent inside the sandbox container.
    /// Run this once before using sandbox mode.
    Auth,
    /// Build the sandbox container image with Claude Code and workmux.
    Build {
        /// Build even on non-Linux OS (workmux binary will not work in image)
        #[arg(long)]
        force: bool,
    },
    /// Delete unused Lima VMs to reclaim disk space.
    Prune {
        /// Skip confirmation and delete all workmux VMs
        #[arg(short, long)]
        force: bool,
    },
}

pub fn run(args: SandboxArgs) -> Result<()> {
    match args.command {
        SandboxCommand::Auth => run_auth(),
        SandboxCommand::Build { force } => run_build(force),
        SandboxCommand::Prune { force } => run_prune(force),
    }
}

fn run_auth() -> Result<()> {
    let config = Config::load(None)?;

    let image_name = config.sandbox.resolved_image();

    println!("Starting sandbox auth flow...");
    println!(
        "This will open Claude in container '{}' for authentication.",
        image_name
    );
    println!("Your credentials will be saved to ~/.claude-sandbox.json\n");

    sandbox::run_auth(&config.sandbox)?;

    println!("\nAuth complete. Sandbox credentials saved.");
    Ok(())
}

fn run_build(force: bool) -> Result<()> {
    let config = Config::load(None)?;

    let image_name = config.sandbox.resolved_image();
    println!("Building sandbox image '{}'...\n", image_name);

    sandbox::build_image(&config.sandbox, force)?;

    println!("\nSandbox image built successfully!");
    println!();
    println!("Enable sandbox in your config:");
    println!();
    println!("  sandbox:");
    println!("    enabled: true");
    if config.sandbox.image.is_none() {
        println!("    # image defaults to 'workmux-sandbox'");
    }
    println!();
    println!("Then authenticate with: workmux sandbox auth");

    Ok(())
}

#[derive(Debug)]
struct VmInfo {
    name: String,
    status: String,
    size_bytes: u64,
    created: Option<SystemTime>,
    last_accessed: Option<SystemTime>,
}

fn run_prune(force: bool) -> Result<()> {
    if !LimaInstance::is_lima_available() {
        bail!("limactl is not installed or not in PATH");
    }

    let output = Command::new("limactl")
        .arg("list")
        .arg("--json")
        .output()
        .context("Failed to execute limactl list")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("Failed to list Lima instances: {}", stderr.trim());
    }

    let instances =
        parse_lima_instances(&output.stdout).context("Failed to parse limactl output")?;

    // Default Lima directory as fallback
    let default_lima_dir = home::home_dir()
        .context("Could not determine home directory")?
        .join(".lima");

    let mut vm_infos: Vec<VmInfo> = Vec::new();

    for instance in instances {
        if !instance.name.starts_with("wm-") {
            continue;
        }

        // Use the dir field from limactl output, fall back to default
        let vm_dir = instance
            .dir
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| default_lima_dir.join(&instance.name));

        let (size_bytes, created, last_accessed) = if vm_dir.exists() {
            let size = calculate_dir_size(&vm_dir)?;
            let metadata = std::fs::metadata(&vm_dir)?;
            let created = metadata.created().ok();
            let last_accessed = metadata.accessed().ok();
            (size, created, last_accessed)
        } else {
            (0, None, None)
        };

        vm_infos.push(VmInfo {
            name: instance.name,
            status: instance.status,
            size_bytes,
            created,
            last_accessed,
        });
    }

    if vm_infos.is_empty() {
        println!("No workmux Lima VMs found.");
        return Ok(());
    }

    // Display VM information
    println!("Found {} workmux Lima VM(s):\n", vm_infos.len());

    let mut total_size: u64 = 0;
    for (i, vm) in vm_infos.iter().enumerate() {
        total_size += vm.size_bytes;

        println!("{}. {} ({})", i + 1, vm.name, vm.status);
        println!("   Size: {}", format_bytes(vm.size_bytes));
        if let Some(created) = vm.created {
            println!("   Age: {}", format_duration_since(created));
        }
        if let Some(accessed) = vm.last_accessed {
            println!("   Last accessed: {}", format_duration_since(accessed));
        }
        println!();
    }

    println!("Total disk space: {}\n", format_bytes(total_size));

    // Confirm deletion unless --force
    if !force {
        print!("Delete all these VMs? [y/N] ");
        io::stdout().flush().context("Failed to flush stdout")?;

        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .context("Failed to read input")?;

        if input.trim().to_lowercase() != "y" {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Delete VMs
    println!("\nDeleting VMs...");
    let mut deleted_count: u64 = 0;
    let mut reclaimed_bytes: u64 = 0;
    let mut failed: Vec<(String, String)> = Vec::new();

    for vm in vm_infos {
        print!("  Deleting {}... ", vm.name);
        io::stdout().flush().ok();

        let result = Command::new("limactl")
            .arg("delete")
            .arg(&vm.name)
            .arg("--force")
            .output();

        match result {
            Ok(output) if output.status.success() => {
                println!("done");
                deleted_count += 1;
                reclaimed_bytes += vm.size_bytes;
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                println!("failed");
                failed.push((vm.name, stderr.trim().to_string()));
            }
            Err(e) => {
                println!("failed");
                failed.push((vm.name, e.to_string()));
            }
        }
    }

    // Report results
    println!();
    if deleted_count > 0 {
        println!(
            "Deleted {} VM(s), reclaimed approximately {}",
            deleted_count,
            format_bytes(reclaimed_bytes)
        );
    }

    if !failed.is_empty() {
        eprintln!("\nFailed to delete {} VM(s):", failed.len());
        for (name, error) in &failed {
            eprintln!("  - {}: {}", name, error);
        }
        bail!("Some VMs could not be deleted");
    }

    Ok(())
}

/// Calculate total size of a directory recursively, without following symlinks.
fn calculate_dir_size(path: &Path) -> Result<u64> {
    let meta = std::fs::symlink_metadata(path)?;

    // Don't follow symlinks
    if meta.file_type().is_symlink() {
        return Ok(0);
    }

    if meta.is_file() {
        return Ok(meta.len());
    }

    let mut total: u64 = 0;
    if meta.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            total += calculate_dir_size(&entry.path())?;
        }
    }

    Ok(total)
}

/// Format bytes as human-readable string.
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];

    if bytes == 0 {
        return "0 B".to_string();
    }

    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", size as u64, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}

/// Format duration since a timestamp as human-readable string.
fn format_duration_since(time: SystemTime) -> String {
    let now = SystemTime::now();

    let duration = match now.duration_since(time) {
        Ok(d) => d,
        Err(_) => return "in the future".to_string(),
    };

    let seconds = duration.as_secs();

    if seconds < 60 {
        return "just now".to_string();
    }

    let minutes = seconds / 60;
    if minutes < 60 {
        return format!(
            "{} minute{} ago",
            minutes,
            if minutes == 1 { "" } else { "s" }
        );
    }

    let hours = minutes / 60;
    if hours < 24 {
        return format!("{} hour{} ago", hours, if hours == 1 { "" } else { "s" });
    }

    let days = hours / 24;
    if days < 30 {
        return format!("{} day{} ago", days, if days == 1 { "" } else { "s" });
    }

    let months = days / 30;
    if months < 12 {
        return format!("{} month{} ago", months, if months == 1 { "" } else { "s" });
    }

    let years = months / 12;
    format!("{} year{} ago", years, if years == 1 { "" } else { "s" })
}
