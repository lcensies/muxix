mod agent;
mod agent_display;
mod bootstrap;
mod claude;
mod cli;
mod cmd;
mod command;
mod config;
mod deps;
mod git;
mod github;
mod llm;
mod logger;
mod markdown;
mod mcp;
mod model;
mod multiplexer;
mod naming;
mod nerdfont;
mod project_state;
mod projects;
mod signals;
mod tasks;
mod prompt;
mod provision;
mod proxy;
mod sandbox;
mod shell;
mod skills;
mod spinner;
mod state;
mod template;
mod tips;
mod tmux_style;
mod tui;
mod ui;
mod util;
mod workflow;
mod xdg;

use anyhow::Result;
use tracing::error;

fn main() -> Result<()> {
    logger::init()?;
    let context = LogContext::current();
    crate::wm_evt!(
        "app.start",
        version = env!("CARGO_PKG_VERSION"),
        args = ?std::env::args().collect::<Vec<_>>(),
        cwd = ?context.cwd,
        pane = ?context.tmux_pane,
    );

    match cli::run() {
        Ok(result) => {
            crate::wm_evt!("app.done");
            Ok(result)
        }
        Err(err) => {
            error!(error = ?err, "workmux failed");
            // Full anyhow chain (Debug renders every `.context()` cause) so the
            // log alone pins the source of the failure.
            crate::wm_evt!("app.fail", err = %err, chain = ?err);
            Err(err)
        }
    }
}

struct LogContext {
    cwd: Option<std::path::PathBuf>,
    tmux_pane: Option<String>,
}

impl LogContext {
    fn current() -> Self {
        Self {
            cwd: std::env::current_dir().ok(),
            tmux_pane: std::env::var("TMUX_PANE").ok(),
        }
    }
}
