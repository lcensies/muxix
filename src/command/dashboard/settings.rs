//! Dashboard settings persistence using StateStore.

use crate::state::StateStore;

/// Load hide_stale filter state from StateStore.
pub fn load_hide_stale() -> bool {
    StateStore::new()
        .ok()
        .and_then(|store| store.load_settings().ok())
        .map(|s| s.hide_stale)
        .unwrap_or(false)
}

/// Save hide_stale filter state to StateStore.
pub fn save_hide_stale(hide_stale: bool) {
    if let Ok(store) = StateStore::new()
        && let Ok(mut settings) = store.load_settings()
    {
        settings.hide_stale = hide_stale;
        let _ = store.save_settings(&settings);
    }
}

/// Load preview size from StateStore.
/// Returns None if not set (so config default can be used).
pub fn load_preview_size() -> Option<u8> {
    StateStore::new()
        .ok()
        .and_then(|store| store.load_settings().ok())
        .and_then(|s| s.preview_size)
}

/// Save preview size to StateStore.
pub fn save_preview_size(size: u8) {
    if let Ok(store) = StateStore::new()
        && let Ok(mut settings) = store.load_settings()
    {
        settings.preview_size = Some(size);
        let _ = store.save_settings(&settings);
    }
}

/// Load last visited pane_id from StateStore.
pub fn load_last_pane_id() -> Option<String> {
    StateStore::new()
        .ok()
        .and_then(|store| store.load_settings().ok())
        .and_then(|s| s.last_pane_id)
}

/// Save last visited pane_id to StateStore.
/// Only saves if value actually changed to minimize disk writes.
pub fn save_last_pane_id(pane_id: &str) {
    if let Ok(store) = StateStore::new()
        && let Ok(mut settings) = store.load_settings()
        && settings.last_pane_id.as_deref() != Some(pane_id)
    {
        settings.last_pane_id = Some(pane_id.to_string());
        let _ = store.save_settings(&settings);
    }
}
