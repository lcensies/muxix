mod claude;
mod cli;
mod cmd;
mod command;
mod config;
mod git;
mod github;
mod llm;
mod logger;
mod naming;
mod prompt;
mod template;
mod tmux;
mod workflow;

use anyhow::Result;
use tracing::{error, info};

fn main() -> Result<()> {
    logger::init()?;
    info!(args = ?std::env::args().collect::<Vec<_>>(), "workmux start");

    match cli::run() {
        Ok(result) => {
            info!("workmux finished successfully");
            Ok(result)
        }
        Err(err) => {
            error!(error = ?err, "workmux failed");
            Err(err)
        }
    }
}
