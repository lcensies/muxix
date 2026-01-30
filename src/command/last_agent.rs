//! Switch to the last visited agent (toggle between two agents).

use anyhow::Result;

use crate::multiplexer::{create_backend, detect_backend};
use crate::state::StateStore;

/// Switch to the last visited agent.
///
/// Reads `last_pane_id` from GlobalSettings and switches to that pane.
/// Updates last_pane_id to the current pane after successful switch.
pub fn run() -> Result<()> {
    let mux = create_backend(detect_backend());
    let store = StateStore::new()?;

    let settings = store.load_settings()?;
    let Some(target_pane_id) = settings.last_pane_id else {
        println!("No previous agent to switch to");
        return Ok(());
    };

    // Get current pane BEFORE switching (this is what becomes "last")
    let current_pane = mux.active_pane_id();

    // Guard: don't switch if already at target (avoids losing history)
    if current_pane.as_deref() == Some(target_pane_id.as_str()) {
        println!("Already at last agent");
        return Ok(());
    }

    // Attempt the switch
    if mux.switch_to_pane(&target_pane_id).is_err() {
        println!("Last agent pane no longer exists");
        return Ok(());
    }

    // Only persist after successful switch
    if let Some(current) = current_pane {
        let mut settings = store.load_settings()?;
        settings.last_pane_id = Some(current);
        store.save_settings(&settings)?;
    }

    Ok(())
}
