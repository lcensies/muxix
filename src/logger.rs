use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, Layer, Registry, fmt, reload};

use crate::sandbox::guest::is_sandbox_guest;

static INIT: OnceLock<()> = OnceLock::new();
static GUARD: OnceLock<WorkerGuard> = OnceLock::new();
/// Handle to the reloadable env filter, so config (`events.level`) can adjust
/// the `muxix::event` level after the subscriber is already installed.
static FILTER_RELOAD: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();

/// Build the base env filter, optionally pinning the `muxix::event` target to
/// `event_level`. `MUXIX_EVENTS` (if set) always wins over `event_level`.
fn build_env_filter(event_level: Option<&str>) -> EnvFilter {
    let mut env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    // Pipeline event tracing (target `muxix::event`) is on by default at `info` and
    // follows the global level otherwise. `MUXIX_EVENTS` overrides just that
    // target — `off` silences it in production, `debug`/`trace` turn on the
    // high-frequency per-poll events — without touching the rest of the filter.
    // Config (`events.level`) is the fallback when `MUXIX_EVENTS` is unset.
    let level = std::env::var("MUXIX_EVENTS")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| event_level.map(|s| s.trim().to_string()))
        .filter(|v| !v.is_empty());

    if let Some(level) = level {
        match format!("{}={}", crate::signals::event::TARGET, level).parse() {
            Ok(directive) => env_filter = env_filter.add_directive(directive),
            Err(e) => eprintln!("warn: ignoring invalid event level {level:?}: {e}"),
        }
    }
    env_filter
}

/// Adjust the `muxix::event` trace level from config after init. No-op if
/// `MUXIX_EVENTS` is set (env wins) or the subscriber was never installed
/// (e.g. sandbox guest). Invalid levels are warned about, not fatal.
pub fn set_event_level(level: &str) {
    if std::env::var("MUXIX_EVENTS")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        return; // env override wins
    }
    let Some(handle) = FILTER_RELOAD.get() else {
        return;
    };
    let new_filter = build_env_filter(Some(level));
    if let Err(e) = handle.reload(new_filter) {
        eprintln!("warn: failed to apply events.level={level:?}: {e}");
    }
}

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

    // Wrap the env filter in a reload layer so config (`events.level`) can adjust
    // the `muxix::event` level after the subscriber is installed. The base level
    // still honours `MUXIX_EVENTS` / `RUST_LOG` first.
    let env_filter = build_env_filter(None);
    let (filter_layer, reload_handle) = reload::Layer::new(env_filter);
    let _ = FILTER_RELOAD.set(reload_handle);

    // Format: JSON lines by default (one object per line — both `jq`-queryable
    // and grep-friendly), so structured event/span fields stay machine-readable.
    // `MUXIX_LOG_FORMAT=text` switches back to the human-readable formatter.
    let text_format = std::env::var("MUXIX_LOG_FORMAT")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "text" | "plain" | "full"
            )
        })
        .unwrap_or(false);

    let layer = if text_format {
        fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .with_target(false)
            .boxed()
    } else {
        // JSON: each line carries `fields` (the event's k/v, incl. `ev`), the
        // enclosing `span`/`spans` context (node, pane, kind), `target`, level
        // and timestamp — so `jq 'select(.fields.ev=="turn.done")'` works and
        // span context (`.spans[].node`) is queryable too.
        fmt::layer()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_writer(non_blocking)
            .with_ansi(false)
            .boxed()
    };

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(layer)
        .try_init()
        .context("Failed to initialize tracing subscriber")?;

    Ok(())
}

fn determine_log_path() -> Result<PathBuf> {
    if let Ok(state_dir) = crate::xdg::state_dir() {
        return Ok(state_dir.join("muxix.log"));
    }

    // Fallback to current directory if home cannot be determined
    Ok(std::env::current_dir()?.join("muxix.log"))
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
