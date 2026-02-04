//! Sandbox management commands.

use anyhow::Result;
use clap::{Args, Subcommand};

use crate::config::Config;
use crate::sandbox;

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
}

pub fn run(args: SandboxArgs) -> Result<()> {
    match args.command {
        SandboxCommand::Auth => run_auth(),
        SandboxCommand::Build { force } => run_build(force),
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
