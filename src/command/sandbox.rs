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
}

pub fn run(args: SandboxArgs) -> Result<()> {
    match args.command {
        SandboxCommand::Auth => run_auth(),
    }
}

fn run_auth() -> Result<()> {
    let config = Config::load(None)?;

    if !config.sandbox.is_enabled() {
        anyhow::bail!(
            "Sandbox is not enabled. Add to your config:\n\n\
             sandbox:\n  \
               enabled: true\n  \
               image: <your-image>"
        );
    }

    println!("Starting sandbox auth flow...");
    println!("This will open the agent in a container for authentication.");
    println!("Your credentials will be saved to ~/.claude-sandbox.json\n");

    sandbox::run_auth(&config.sandbox)?;

    println!("\nAuth complete. Sandbox credentials saved.");
    Ok(())
}
