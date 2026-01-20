use anyhow::Result;

use crate::config::Config;
use crate::tmux;

/// Switch to the agent that most recently completed its task.
///
/// Finds all panes with "done" status and switches to the one with the most
/// recent timestamp. Prints a message if no completed agents are found.
pub fn run() -> Result<()> {
    let config = Config::load(None)?;
    let done_icon = config.status_icons.done();

    if tmux::switch_to_last_completed(done_icon)? {
        // Success - the switch itself is the feedback
        Ok(())
    } else {
        println!("No completed agents found");
        Ok(())
    }
}
