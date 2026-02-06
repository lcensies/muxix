use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, fmt};

use crate::sandbox::guest::is_sandbox_guest;

static INIT: OnceLock<()> = OnceLock::new();
static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

pub fn init() -> Result<()> {
    if INIT.get().is_some() {
        return Ok(());
    }

    // Skip file logging in sandbox guests - they're thin RPC clients and the
    // host supervisor handles all real logging. Also avoids needing to create
    // ~/.local/state/ in containers.
    if is_sandbox_guest() {
        let _ = INIT.set(());
        return Ok(());
    }

    init_inner()?;
    let _ = INIT.set(());
    Ok(())
}

fn init_inner() -> Result<()> {
    let log_path = determine_log_path()?;
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create log directory at {}", parent.display()))?;
    }

    let (directory, file_name) = split_path(&log_path)?;
    let file_appender = rolling::never(directory, file_name);
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
    let _ = GUARD.set(guard);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(env_filter)
        .with(
            fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false)
                .with_target(false),
        )
        .try_init()
        .context("Failed to initialize tracing subscriber")?;

    Ok(())
}

fn determine_log_path() -> Result<PathBuf> {
    // Check XDG_STATE_HOME environment variable first
    if let Ok(state_home) = std::env::var("XDG_STATE_HOME")
        && !state_home.is_empty()
    {
        return Ok(PathBuf::from(state_home)
            .join("workmux")
            .join("workmux.log"));
    }

    // Fall back to XDG default: ~/.local/state/workmux/workmux.log
    if let Some(home_dir) = home::home_dir() {
        return Ok(home_dir
            .join(".local")
            .join("state")
            .join("workmux")
            .join("workmux.log"));
    }

    // Fallback to current directory if home cannot be determined
    Ok(std::env::current_dir()?.join("workmux.log"))
}

fn split_path(path: &Path) -> Result<(PathBuf, &str)> {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("Invalid log file name"))?;

    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    Ok((dir, file_name))
}
