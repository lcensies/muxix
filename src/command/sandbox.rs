//! Sandbox management commands.

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;
use tracing::debug;

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
    /// Run a command inside a Lima sandbox (internal, used by pane setup).
    #[command(hide = true)]
    Run {
        /// Path to the worktree
        worktree: PathBuf,
        /// Command and arguments to run inside the sandbox
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Cross-compile and install workmux into running Lima VMs for development.
    InstallDev {
        /// Skip cross-compilation and use existing binary
        #[arg(long)]
        skip_build: bool,
        /// Use release profile (slower build, faster binary)
        #[arg(long)]
        release: bool,
    },
    /// Stop Lima VMs to free resources.
    Stop {
        /// VM name to stop (if not provided, show interactive list)
        #[arg(conflicts_with = "all")]
        name: Option<String>,
        /// Stop all workmux VMs (wm-* prefix)
        #[arg(long)]
        all: bool,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
}

pub fn run(args: SandboxArgs) -> Result<()> {
    match args.command {
        SandboxCommand::Auth => run_auth(),
        SandboxCommand::Build { force } => run_build(force),
        SandboxCommand::Run { worktree, command } => {
            debug!(worktree = %worktree.display(), ?command, "sandbox run");
            let exit_code = super::sandbox_run::run(worktree, command)?;
            std::process::exit(exit_code);
        }
        SandboxCommand::InstallDev {
            skip_build,
            release,
        } => run_install_dev(skip_build, release),
        SandboxCommand::Prune { force } => run_prune(force),
        SandboxCommand::Stop { name, all, yes } => run_stop(name, all, yes),
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

fn linux_target_triple() -> Result<&'static str> {
    match std::env::consts::ARCH {
        "aarch64" => Ok("aarch64-unknown-linux-gnu"),
        "x86_64" => Ok("x86_64-unknown-linux-gnu"),
        arch => bail!(
            "unsupported host architecture for cross-compilation: {}",
            arch
        ),
    }
}

fn find_cargo_workspace() -> Result<PathBuf> {
    let output = Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()
        .context("Failed to run cargo locate-project")?;
    if !output.status.success() {
        bail!("Failed to locate cargo workspace");
    }
    let path = String::from_utf8_lossy(&output.stdout);
    let cargo_toml = PathBuf::from(path.trim());
    cargo_toml
        .parent()
        .map(|p| p.to_path_buf())
        .context("Failed to determine workspace root from Cargo.toml path")
}

fn cross_compile(target: &str, release: bool) -> Result<PathBuf> {
    // Check if target is installed
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .context("Failed to run rustup")?;
    let installed = String::from_utf8_lossy(&output.stdout);
    if !installed.lines().any(|l| l.trim() == target) {
        bail!(
            "Rust target {} is not installed.\n\
             Install it with: rustup target add {}",
            target,
            target
        );
    }

    // Check if cross-linker is available (unless CARGO_TARGET_*_LINKER is set)
    let linker_env = format!(
        "CARGO_TARGET_{}_LINKER",
        target.to_uppercase().replace('-', "_")
    );
    let linker_set = std::env::var(&linker_env).is_ok();
    if !linker_set && which::which(format!("{}-gcc", target)).is_err() {
        bail!(
            "Cross-linker {}-gcc not found and {} is not set.\n\
             Install with: brew install messense/macos-cross-toolchains/{}",
            target,
            linker_env,
            target,
        );
    }

    let workspace = find_cargo_workspace()?;
    let profile = if release { "release" } else { "debug" };
    let profile_dir = if release { "release" } else { "debug" };

    println!("Cross-compiling workmux for {} ({})...\n", target, profile);

    let mut cmd = Command::new("cargo");
    cmd.args(["build", "--target", target]);
    if release {
        cmd.arg("--release");
    }
    cmd.current_dir(&workspace);

    // Set linker env var if not already set
    if !linker_set {
        cmd.env(&linker_env, format!("{}-gcc", target));
    }

    let status = cmd.status().context("Failed to run cargo build")?;
    if !status.success() {
        bail!("Cross-compilation failed");
    }

    let binary = workspace.join(format!("target/{}/{}/workmux", target, profile_dir));
    if !binary.exists() {
        bail!("Expected binary not found at {}", binary.display());
    }

    println!();
    Ok(binary)
}

fn install_to_vm(binary_path: &Path, vm_name: &str) -> Result<()> {
    // Use bash -c with $HOME for absolute paths because limactl shell sets
    // the working directory to the host cwd (if mounted), not $HOME.
    let mkdir = Command::new("limactl")
        .args([
            "shell",
            vm_name,
            "--",
            "bash",
            "-c",
            "mkdir -p \"$HOME/.local/bin\"",
        ])
        .output()
        .context("Failed to run limactl shell for mkdir")?;
    if !mkdir.status.success() {
        let stderr = String::from_utf8_lossy(&mkdir.stderr);
        bail!("Failed to create ~/.local/bin: {}", stderr.trim());
    }

    // Copy to temp location to avoid "text file busy"
    let tmp_dest = format!("{}:/tmp/workmux.new", vm_name);
    let cp = Command::new("limactl")
        .args(["cp", &binary_path.to_string_lossy(), &tmp_dest])
        .output()
        .context("Failed to run limactl cp")?;
    if !cp.status.success() {
        let stderr = String::from_utf8_lossy(&cp.stderr);
        bail!("Failed to copy binary: {}", stderr.trim());
    }

    // Move into place and make executable
    let install = Command::new("limactl")
        .args([
            "shell",
            vm_name,
            "--",
            "bash",
            "-c",
            "install -m 755 /tmp/workmux.new \"$HOME/.local/bin/workmux\"",
        ])
        .output()
        .context("Failed to run limactl shell for install")?;
    if !install.status.success() {
        let stderr = String::from_utf8_lossy(&install.stderr);
        bail!("Failed to install binary: {}", stderr.trim());
    }

    Ok(())
}

fn run_install_dev(skip_build: bool, release: bool) -> Result<()> {
    use crate::sandbox::lima::VM_PREFIX;

    if !LimaInstance::is_lima_available() {
        bail!("limactl is not installed or not in PATH");
    }

    let target = linux_target_triple()?;

    // Check for running VMs first (before potentially slow compilation)
    let instances = LimaInstance::list()?;
    let running: Vec<_> = instances
        .iter()
        .filter(|i| i.name.starts_with(VM_PREFIX) && i.is_running())
        .collect();

    if running.is_empty() {
        println!("No running workmux VMs found.");
        return Ok(());
    }

    // Cross-compile (or locate existing binary)
    let binary_path = if !skip_build {
        cross_compile(target, release)?
    } else {
        let workspace = find_cargo_workspace()?;
        let profile_dir = if release { "release" } else { "debug" };
        let path = workspace.join(format!("target/{}/{}/workmux", target, profile_dir));
        if !path.exists() {
            bail!(
                "No cross-compiled binary found at {}\nRun without --skip-build first.",
                path.display()
            );
        }
        path
    };

    // Install into each running VM
    println!(
        "Installing workmux into {} running VM(s)...\n",
        running.len()
    );
    let mut failed: Vec<(String, String)> = Vec::new();

    for vm in &running {
        print!("  {} ... ", vm.name);
        io::stdout().flush().ok();

        match install_to_vm(&binary_path, &vm.name) {
            Ok(()) => println!("ok"),
            Err(e) => {
                println!("failed");
                failed.push((vm.name.clone(), e.to_string()));
            }
        }
    }

    if !failed.is_empty() {
        eprintln!("\nFailed to install to {} VM(s):", failed.len());
        for (name, error) in &failed {
            eprintln!("  - {}: {}", name, error);
        }
        bail!("Some installations failed");
    }

    println!("\nDone.");
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

fn run_stop(name: Option<String>, all: bool, skip_confirm: bool) -> Result<()> {
    use crate::sandbox::lima::{LimaInstance, LimaInstanceInfo, VM_PREFIX};
    use std::io::{self, IsTerminal, Write};

    // Check if limactl is available
    if !LimaInstance::is_lima_available() {
        anyhow::bail!("limactl not found. Please install Lima first.");
    }

    // Get list of all workmux VMs
    let all_vms = LimaInstance::list()?;
    let workmux_vms: Vec<LimaInstanceInfo> = all_vms
        .into_iter()
        .filter(|vm| vm.name.starts_with(VM_PREFIX))
        .collect();

    // Filter to running VMs for display/selection
    let running_vms: Vec<&LimaInstanceInfo> =
        workmux_vms.iter().filter(|vm| vm.is_running()).collect();

    let vms_to_stop: Vec<&LimaInstanceInfo> = if all {
        // Stop all running VMs
        if running_vms.is_empty() {
            println!("No running workmux VMs found.");
            return Ok(());
        }
        running_vms
    } else if let Some(ref vm_name) = name {
        // Stop specific VM - check all VMs (not just running) for better error messages
        let vm = workmux_vms.iter().find(|v| v.name == *vm_name);
        match vm {
            Some(v) if v.is_running() => vec![v],
            Some(v) => {
                println!(
                    "VM '{}' is already stopped (status: {}).",
                    vm_name, v.status
                );
                return Ok(());
            }
            None => {
                anyhow::bail!(
                    "VM '{}' not found. Use 'workmux sandbox stop' to see available VMs.",
                    vm_name
                );
            }
        }
    } else {
        // Interactive mode: require TTY
        if !std::io::stdin().is_terminal() {
            anyhow::bail!("Non-interactive stdin detected. Use --all or specify a VM name.");
        }

        if running_vms.is_empty() {
            println!("No running workmux VMs found.");
            return Ok(());
        }

        select_vms_interactive(&running_vms)?
    };

    if vms_to_stop.is_empty() {
        println!("No VMs selected.");
        return Ok(());
    }

    // Show what will be stopped
    println!("The following VMs will be stopped:");
    for vm in &vms_to_stop {
        println!("  - {} ({})", vm.name, vm.status);
    }

    // Confirm unless --yes flag is provided
    if !skip_confirm {
        print!(
            "\nAre you sure you want to stop {} VM(s)? [y/N] ",
            vms_to_stop.len()
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let answer = input.trim().to_ascii_lowercase();
        if !matches!(answer.as_str(), "y" | "yes") {
            println!("Aborted.");
            return Ok(());
        }
    }

    // Stop VMs
    let mut success_count = 0;
    let mut failed: Vec<(String, String)> = Vec::new();

    for vm in vms_to_stop {
        print!("Stopping {}... ", vm.name);
        io::stdout().flush()?;

        match LimaInstance::stop_by_name(&vm.name) {
            Ok(()) => {
                println!("✓");
                success_count += 1;
            }
            Err(e) => {
                println!("✗");
                failed.push((vm.name.clone(), e.to_string()));
            }
        }
    }

    // Report results
    if success_count > 0 {
        println!("\n✓ Successfully stopped {} VM(s)", success_count);
    }

    if !failed.is_empty() {
        eprintln!("\nFailed to stop {} VM(s):", failed.len());
        for (name, error) in &failed {
            eprintln!("  - {}: {}", name, error);
        }
        anyhow::bail!("Some VMs could not be stopped");
    }

    Ok(())
}

fn select_vms_interactive<'a>(
    vms: &'a [&'a crate::sandbox::lima::LimaInstanceInfo],
) -> Result<Vec<&'a crate::sandbox::lima::LimaInstanceInfo>> {
    use std::io::{self, Write};

    println!("Running workmux VMs:");
    println!();
    for (idx, vm) in vms.iter().enumerate() {
        println!("  {}. {} ({})", idx + 1, vm.name, vm.status);
    }
    println!();
    println!("Enter VM number to stop (or 'all' for all VMs):");
    print!("> ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.eq_ignore_ascii_case("all") {
        return Ok(vms.to_vec());
    }

    // Parse as number
    let idx: usize = input
        .parse()
        .context("Invalid input. Please enter a number or 'all'.")?;

    if idx < 1 || idx > vms.len() {
        anyhow::bail!(
            "Invalid selection. Please choose a number between 1 and {}",
            vms.len()
        );
    }

    Ok(vec![vms[idx - 1]])
}
