# Zellij Dashboard Persistence Fix - Implementation Summary

## Problem

Agents running in Zellij did not persist in the dashboard. They briefly appeared when focused, then quickly disappeared when unfocused. This was caused by Zellij's CLI limitation: `zellij action list-clients` only returns information about the currently focused pane.

## Solution Architecture

Implemented a **trait-based backend-specific validation** approach that keeps all Zellij-specific logic isolated in `zellij.rs` while maintaining clean, multiplexer-agnostic code in shared modules.

## Changes Made

### 1. Added `validate_agent_alive` to Multiplexer Trait

**File:** `src/multiplexer/mod.rs`

- New trait method `validate_agent_alive(&self, state: &AgentState) -> Result<bool>`
- Default implementation uses PID and command validation (for tmux/WezTerm)
- Backends can override with custom logic

### 2. Extended AgentState with window/session fields

**File:** `src/state/types.rs`

Added fields to store window and session names:
```rust
pub struct AgentState {
    // ... existing fields ...

    /// Window/tab name where this agent is running
    #[serde(default)]  // Backward compatible
    pub window_name: Option<String>,

    /// Session name where this agent is running
    #[serde(default)]  // Backward compatible
    pub session_name: Option<String>,
}
```

**Rationale:** Since Zellij can't query unfocused panes, we store the window name when the agent is focused (during `set-window-status` call). This allows us to validate tab existence during reconciliation.

### 3. Modified Zellij's `get_live_pane_info`

**File:** `src/multiplexer/zellij.rs`

- Now returns fallback data for unfocused panes instead of `None`
- Uses `std::env::current_dir()` for working directory
- Allows state persistence even when pane loses focus

### 4. Implemented Zellij-specific `validate_agent_alive`

**File:** `src/multiplexer/zellij.rs`

Tab-level validation logic:
1. **Tab existence check:** Uses stored `window_name` to verify the tab still exists
2. **Staleness check:** Removes agents with no updates for > 1 hour

```rust
fn validate_agent_alive(&self, state: &AgentState) -> Result<bool> {
    // Check if tab exists
    if let Some(window_name) = &state.window_name {
        if !self.window_exists_by_full_name(window_name)? {
            return Ok(false); // Tab closed
        }
    }

    // Check staleness (1 hour threshold)
    if state_age > 3600 seconds {
        return Ok(false); // Stale
    }

    Ok(true)
}
```

### 5. Updated State Storage

**File:** `src/command/set_window_status.rs`

Now stores `window_name` and `session_name` from `LivePaneInfo` when creating agent state.

### 6. Simplified Reconciliation Logic

**File:** `src/state/store.rs`

Replaced complex pattern matching with single trait method call:

```rust
// Old approach: Manual PID/command checking
match live_pane {
    None => delete,
    Some(live) if pid_mismatch => delete,
    Some(live) if command_changed => delete,
    Some(live) => keep,
}

// New approach: Backend-specific validation
if mux.validate_agent_alive(&state)? {
    keep_agent()
} else {
    delete_agent()
}
```

## Benefits

✅ **Architecture:** All Zellij-specific logic isolated to `zellij.rs`
✅ **Maintainability:** Generic code remains multiplexer-agnostic
✅ **Extensibility:** Easy to add new backends with different capabilities
✅ **Backward Compatibility:** Old state files work (window_name defaults to None)
✅ **Polymorphism:** Proper use of trait methods for backend-specific behavior

## Behavior

### Zellij
- **Persistence:** Agents persist in dashboard when unfocused
- **Cleanup:** Removed when tab closes OR after 1 hour of inactivity
- **Limitation:** Can't detect agent exit within a tab (only tab-level tracking)

### tmux/WezTerm (unchanged)
- **Persistence:** Full pane-level tracking
- **Cleanup:** Removed when PID changes OR foreground command changes
- **Validation:** Accurate process-level detection

## Testing

All 231 tests pass, including:
- State serialization/deserialization with new fields
- Backward compatibility with old state files
- Agent reconciliation logic

## Known Limitations for Zellij

1. **Staleness timeout:** Agents inactive for > 1 hour are cleaned up
   - Mitigation: Long-running tasks should periodically call `workmux set-window-status`

2. **No process-level tracking:** Can't detect when agent exits to shell within the same tab
   - Mitigation: Tab closure or staleness timeout will eventually clean up

3. **Old state files:** Files without `window_name` are conservatively kept
   - Mitigation: Manual cleanup via dashboard if needed

## Files Modified

1. `src/multiplexer/mod.rs` - Added `validate_agent_alive` trait method
2. `src/multiplexer/zellij.rs` - Implemented Zellij-specific validation
3. `src/state/types.rs` - Added window_name/session_name fields
4. `src/command/set_window_status.rs` - Store window/session names
5. `src/state/store.rs` - Simplified reconciliation logic
6. Tests updated to include new fields

## Migration Path

No migration needed - the implementation is fully backward compatible:
- `#[serde(default)]` on new fields ensures old state files deserialize correctly
- Validation logic handles missing window_name gracefully
