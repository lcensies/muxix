# Zellij Dashboard Agent Persistence Issue

**Status:** ✅ RESOLVED - See [ZELLIJ_FIX_IMPLEMENTATION.md](./ZELLIJ_FIX_IMPLEMENTATION.md) for implementation details

## Problem Statement

Agents running in Zellij do not persist in the `workmux dashboard`. They briefly appear when focused, then quickly disappear when unfocused. This makes the dashboard unusable for monitoring multiple agents in Zellij.

## Root Cause

### Technical Details

The issue stems from Zellij's CLI limitations compared to tmux:

1. **State Persistence Flow:**
   - Claude Code calls `workmux set-window-status working`
   - workmux calls `mux.get_live_pane_info()` to get pane metadata (PID, command, working dir)
   - This metadata is saved to `~/.local/state/workmux/agents/<pane>.json`
   - Dashboard reads these state files to display agents

2. **Zellij's `get_live_pane_info` Implementation Limitation:**
   - Location: `src/multiplexer/zellij.rs:663-691`
   - Uses `zellij action list-clients` to get pane info
   - **Problem:** This command only returns the currently focused pane
   - For unfocused panes: returns `None`

3. **The Failure Cycle:**
   ```
   Agent is FOCUSED:
   ├─> get_live_pane_info() returns Some(...)
   ├─> State file created in ~/.local/state/workmux/agents/
   └─> Agent appears in dashboard ✓

   Agent becomes UNFOCUSED:
   ├─> Dashboard reconciliation runs
   ├─> get_live_pane_info() returns None
   ├─> Reconciliation thinks pane no longer exists
   ├─> State file deleted
   └─> Agent disappears from dashboard ✗
   ```

### Why This Affects Zellij Specifically

**tmux:**
- Can query any pane: `tmux display-message -t <pane-id> -p <format>`
- Returns info for focused AND unfocused panes

**WezTerm:**
- Has `wezterm cli list --format json`
- Returns all panes with their metadata

**Zellij:**
- `zellij action list-clients` only shows focused pane
- `zellij action dump-layout` shows tab structure but not runtime state (PID, current command, working dir)

### Evidence

- Empty agents directory: `~/.local/state/workmux/agents/` has no files
- Logs show successful `set-window-status` calls but no persistence
- `zellij action list-clients` output shows only 1 pane (the focused one)

## Proposed Solutions

### Option 1: Fallback LivePaneInfo for Unfocused Panes

**Approach:** Return dummy/cached info for unfocused panes instead of `None`

**Implementation:**
```rust
// src/multiplexer/zellij.rs:663
fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>> {
    let clients = Self::list_clients()?;

    // Try to get real info for focused pane
    for client in clients {
        if client.pane_id == pane_id {
            return Ok(Some(LivePaneInfo {
                pid: 0,
                current_command: client.running_command.split_whitespace().next().unwrap_or("").to_string(),
                working_dir: std::env::current_dir().unwrap_or_default(),
                title: None,
                session: Self::session_name(),
                window: Self::focused_tab_name(),
            }));
        }
    }

    // NEW: Return fallback for unfocused panes
    // Prevents reconciliation from deleting state
    Ok(Some(LivePaneInfo {
        pid: 0,
        current_command: String::new(),
        working_dir: PathBuf::new(),
        title: None,
        session: Self::session_name(),
        window: None, // Don't know which tab for unfocused panes
    }))
}
```

**Pros:**
- Minimal code changes
- Agents persist in dashboard
- Works with existing reconciliation logic

**Cons:**
- Inaccurate data for unfocused panes (no PID, no working dir)
- Reconciliation can't detect if agent actually exited (PID always 0)
- Can't detect command changes (stale agents won't be cleaned up)
- `window` name will be wrong for unfocused tabs

**Risk:** Dead agents (actually exited) won't be removed from dashboard

---

### Option 2: Zellij-Specific State Tracking (Tab-Level)

**Approach:** Special-case Zellij to track state at tab level, not pane level

**Implementation:**
```rust
// src/command/set_window_status.rs:65
if mux.name() == "zellij" {
    // Zellij: Use environment/context since we can't query unfocused panes
    let state = AgentState {
        pane_key,
        workdir: std::env::current_dir()?,
        status: Some(status),
        status_ts: Some(now),
        pane_title: std::env::var("ZELLIJ_SESSION_NAME").ok(),
        pane_pid: std::process::id(),
        command: std::env::var("WORKMUX_AGENT").unwrap_or_else(|_| "claude".to_string()),
        updated_ts: now,
    };

    if let Ok(store) = StateStore::new() {
        let _ = store.upsert_agent(&state);
    }
} else {
    // tmux/wezterm: Use live pane info
    if let Ok(Some(live_info)) = mux.get_live_pane_info(&pane_id) {
        // ...existing code...
    }
}
```

**Reconciliation changes:**
```rust
// src/state/store.rs - modify load_reconciled_agents for Zellij
if state.pane_key.backend == "zellij" {
    // For Zellij: Only check if tab still exists
    // Can't validate PID or command changes
    let tabs = /* query tab names */;
    if tabs.contains(&window_name) {
        valid_agents.push(agent_pane);
    } else {
        self.delete_agent(&state.pane_key)?;
    }
} else {
    // tmux/wezterm: Full reconciliation with PID/command checks
    // ...existing code...
}
```

**Pros:**
- Agents persist reliably
- Uses accurate data we CAN get (cwd from env, process PID)
- Clear separation of Zellij's limitations

**Cons:**
- Special-case code path for Zellij
- Can't detect agent exit within a pane (only tab close)
- Less accurate reconciliation (can't tell if claude exited to shell)

**Risk:** If agent crashes but pane/tab stays open, stale state persists until tab is closed

---

### Option 3: Environment-Based Working Directory (Cross-Platform)

**Approach:** Get working directory from calling process instead of multiplexer query

**Implementation:**
```rust
// src/command/set_window_status.rs:65
// For ALL backends: Use environment for working directory
let workdir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));

if let Ok(Some(live_info)) = mux.get_live_pane_info(&pane_id) {
    let state = AgentState {
        pane_key,
        workdir, // From environment, not live_info
        status: Some(status),
        status_ts: Some(now),
        pane_title: live_info.title,
        pane_pid: live_info.pid,
        command: live_info.current_command,
        updated_ts: now,
    };
    // ...
} else if mux.name() == "zellij" {
    // Zellij fallback: Limited info but still persist
    let state = AgentState {
        pane_key,
        workdir,
        status: Some(status),
        status_ts: Some(now),
        pane_title: None,
        pane_pid: 0, // Can't get for unfocused panes
        command: String::new(),
        updated_ts: now,
    };
    // ...
}
```

**Pros:**
- Working directory is always accurate (from the calling process)
- Hybrid approach: full data for tmux/wezterm, partial for Zellij
- Simpler than Option 2

**Cons:**
- Still has Zellij-specific fallback code
- Zellij still can't do proper reconciliation (PID=0, command empty)

---

### Option 4: Disable Reconciliation for Zellij

**Approach:** Skip PID/command validation for Zellij, only check tab existence

**Implementation:**
```rust
// src/state/store.rs:162
if state.pane_key.backend == "zellij" {
    // Zellij: Simple tab existence check
    if mux.window_exists_by_full_name(&window_name)? {
        let agent_pane = state.to_agent_pane(
            Self::session_name().unwrap_or_default(),
            window_name,
        );
        valid_agents.push(agent_pane);
    } else {
        // Tab closed, clean up
        self.delete_agent(&state.pane_key)?;
    }
} else {
    // tmux/wezterm: Full reconciliation
    let live_pane = mux.get_live_pane_info(&state.pane_key.pane_id)?;
    // ...existing validation...
}
```

**Pros:**
- Clean separation: Zellij uses tab-level tracking
- No dummy data (PID/command not used for Zellij)
- Agents persist as long as tab exists

**Cons:**
- Zellij loses fine-grained reconciliation
- Dead agents persist if tab remains open
- Dashboard may show stale agents

---

## Recommendation

**Option 4** (Disable Reconciliation for Zellij) is the cleanest approach:

1. **Acknowledges Zellij's limitations explicitly** rather than working around them with dummy data
2. **Tab-level tracking matches Zellij's architecture** (status is tracked per tab, not per pane - see zellij.rs:8)
3. **No risk of misleading data** (PID=0 or empty commands could mask real issues)
4. **Simpler code** than Options 2-3
5. **Users can manually clean up stale agents** via dashboard if needed

### Implementation Plan

1. Modify `src/command/set_window_status.rs`:
   - For Zellij: Always create state using environment data
   - Don't depend on `get_live_pane_info` success

2. Modify `src/state/store.rs::load_reconciled_agents`:
   - Add Zellij-specific reconciliation path
   - Only check tab existence, skip PID/command validation

3. Document the limitation in dashboard docs:
   - "Note: Zellij backend uses tab-level tracking. Agents are cleaned up when tabs close, but not when the agent process exits within a tab."

### Alternative: Option 2 (Acceptable)

If you want more control and explicit Zellij handling throughout, Option 2 is also solid. It's more verbose but makes the special case very clear.

## Testing Checklist

After implementing the fix:

- [ ] Create agent in Zellij tab
- [ ] Verify state file created in `~/.local/state/workmux/agents/`
- [ ] Switch to different tab
- [ ] Open dashboard - agent should still be visible
- [ ] Close the agent's tab
- [ ] Verify state file is deleted
- [ ] Verify agent removed from dashboard

## Related Files

- `src/multiplexer/zellij.rs:663-691` - `get_live_pane_info` implementation
- `src/command/set_window_status.rs:65` - Agent state creation
- `src/state/store.rs:148-194` - Agent reconciliation logic
- `src/state/types.rs` - AgentState structure
