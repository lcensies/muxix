use anyhow::{Result, anyhow};

use crate::multiplexer::{create_backend, detect_backend};
use crate::workflow;

pub fn run(name: &str, lines: u16) -> Result<()> {
    let mux = create_backend(detect_backend());
    let (_path, agent) = workflow::resolve_worktree_agent(name, mux.as_ref())?;

    let output = mux
        .capture_pane(&agent.pane_id, lines)
        .ok_or_else(|| anyhow!("Failed to capture pane output"))?;

    // Strip ANSI escape codes
    let stripped = strip_ansi_escapes::strip_str(&output);
    print!("{stripped}");

    Ok(())
}
