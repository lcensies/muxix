use anyhow::Result;

use crate::tmux;

/// Switch to the agent that most recently completed its task.
///
/// Uses a persistent stack of done panes stored in a tmux server variable
/// for fast lookups. Cycles through completed agents on repeated invocations.
pub fn run() -> Result<()> {
    if tmux::switch_to_last_completed()? {
        // Success - the switch itself is the feedback
        Ok(())
    } else {
        println!("No completed agents found");
        Ok(())
    }
}
