//! Switch to the last visited agent (toggle between two agents).

use anyhow::Result;

use crate::multiplexer::{create_backend, detect_backend};
use crate::state::StateStore;

/// Switch to the last visited agent.
///
/// Reads `last_pane_id` from GlobalSettings and switches to that pane.
/// Updates last_pane_id to the current pane after successful switch,
/// but only if the current pane is also an agent pane.
pub fn run() -> Result<()> {
    let mux = create_backend(detect_backend());
    let store = StateStore::new()?;

    // Load agents to verify panes are actually agent panes
    let agents = store
        .load_reconciled_agents(mux.as_ref())
        .unwrap_or_default();

    let settings = store.load_settings()?;
    let Some(target_pane_id) = settings.last_pane_id else {
        println!("No previous agent to switch to");
        return Ok(());
    };

    // Verify target is still an agent pane
    if !agents.iter().any(|a| a.pane_id == target_pane_id) {
        println!("Last agent pane no longer exists");
        return Ok(());
    }

    // Get current pane BEFORE switching (this is what becomes "last")
    let current_pane = mux.active_pane_id();

    // Guard: don't switch if already at target (avoids losing history)
    if current_pane.as_deref() == Some(target_pane_id.as_str()) {
        println!("Already at last agent");
        return Ok(());
    }

    // Attempt the switch
    let window_hint = agents
        .iter()
        .find(|a| a.pane_id == target_pane_id)
        .map(|a| a.window_name.as_str());
    if mux.switch_to_pane(&target_pane_id, window_hint).is_err() {
        println!("Failed to switch to last agent");
        return Ok(());
    }

    // Only persist after successful switch, and only if current pane is an agent
    if let Some(ref current) = current_pane
        && agents.iter().any(|a| a.pane_id == *current)
    {
        let mut settings = store.load_settings()?;
        settings.last_pane_id = Some(current.clone());
        store.save_settings(&settings)?;
    }

    Ok(())
}
