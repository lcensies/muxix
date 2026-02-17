# Zellij Multiplexer Improvements: Upstream Integration Plan

## Context

Workmux recently added zellij support with several workarounds due to API limitations compared to tmux/WezTerm. Recent upstream zellij commits introduce critical capabilities that eliminate most workarounds:

**Upstream Commits:**
- **13a82ee**: Adds `list-tabs`, `current-tab-info` CLI commands and tab ID-based operations
- **e160e9e**: Adds `WriteToPaneId` and `WriteCharsToPaneId` - ability to write to specific panes by ID

**Current Limitations** (from `src/multiplexer/zellij.rs:141-148`):
```rust
pane_targeting: false,    // Commands only go to focused pane
stable_pane_ids: false,   // Using tab names as fake pane IDs
supports_preview: false,  // Expensive process spawning
```

**Impact**: These limitations require ~150 lines of custom workarounds including manual focus navigation (lines 662-693), tab-level validation instead of pane-level (lines 806-858), and ignoring pane_id parameters in send methods (lines 435-447).

**Goal**: Integrate new upstream APIs to achieve ~90% feature parity with tmux/WezTerm, removing architectural limitations and code workarounds.

## Critical Files

**Primary modification:**
- `src/multiplexer/zellij.rs` (lines 1-953) - All implementation changes

**Reference files for patterns:**
- `src/multiplexer/mod.rs` (lines 205-392) - Default `setup_panes()` implementation
- `src/multiplexer/tmux.rs` (lines 428-479) - Pane targeting pattern reference
- `src/state/types.rs` (lines 68-117) - AgentState structure for validation

## Implementation Phases

### Phase 1: Pane ID Infrastructure (CRITICAL - Foundation)

**Objective**: Switch from tab-name-based fake pane IDs to real numeric zellij pane IDs.

**Changes to `src/multiplexer/zellij.rs`:**

1. **Add pane query structures** (after line 34):
   ```rust
   #[derive(Debug, serde::Deserialize)]
   struct PaneInfo {
       id: u32,
       is_plugin: bool,
       is_focused: bool,
       terminal_command: Option<String>,
       tab_name: String,
       title: String,
   }

   #[derive(Debug, serde::Deserialize)]
   struct TabInfo {
       tab_id: u32,
       name: String,
       position: usize,
       active: bool,
   }
   ```

2. **Add query methods** (after line 133):
   ```rust
   /// Query all panes using `zellij action list-panes --json`
   fn list_panes() -> Result<Vec<PaneInfo>>

   /// Query all tabs using `zellij action list-tabs --json`
   fn list_tabs() -> Result<Vec<TabInfo>>

   /// Get focused pane ID from list-panes output
   fn focused_pane_id() -> Result<u32>
   ```

3. **Update pane ID methods** (lines 162-171):
   - `current_pane_id()`: Try env var first (fast path)
   - `active_pane_id()`: Query focused pane (reliable)
   - Format: `"terminal_{id}"` for consistency

4. **Update `create_window()`** (lines 187-208):
   - After creating tab, query focused pane ID
   - Return `format!("terminal_{}", pane_id)` instead of tab name

5. **Update `split_pane()`** (lines 708-764, specifically 759-763):
   - After split, query focused pane ID (new pane)
   - Return real pane ID instead of placeholder

**Verification**: State files should contain IDs like `"terminal_0"`, `"terminal_1"` instead of tab names.

---

### Phase 2: Pane Targeting (HIGH - Core Feature)

**Objective**: Use `--pane-id` flag to send commands to specific panes instead of only focused pane.

**Changes to `src/multiplexer/zellij.rs`:**

1. **Update `send_keys()`** (lines 435-447):
   ```rust
   fn send_keys(&self, pane_id: &str, command: &str) -> Result<()> {
       Cmd::new("zellij")
           .args(&["action", "write-chars", "--pane-id", pane_id, command])
           .run()?;
       Cmd::new("zellij")
           .args(&["action", "write", "--pane-id", pane_id, "13"])
           .run()?;
       Ok(())
   }
   ```

2. **Update `send_key()`** (lines 475-496):
   - Add `--pane-id` to both `write-chars` and `write` calls

3. **Update `paste_multiline()`** (lines 498-507):
   - Add `--pane-id` to line-by-line write operations

4. **Update `send_keys_to_agent()`** (lines 450-473):
   - Add `--pane-id` to bang delay workaround logic

5. **Update capabilities** (lines 141-148):
   ```rust
   fn capabilities(&self) -> super::MultiplexerCaps {
       super::MultiplexerCaps {
           pane_targeting: true,     // Now supported
           stable_pane_ids: true,    // Numeric IDs are stable
           supports_preview: false,
           exit_on_jump: false,
       }
   }
   ```

**Verification**: Create multi-pane layout, send commands to unfocused panes, verify execution.

---

### Phase 3: Simplified Setup (MEDIUM - Cleanup)

**Objective**: Remove custom `setup_panes()` override, use default trait implementation.

**Changes to `src/multiplexer/zellij.rs`:**

1. **Delete custom `setup_panes()`** (lines 549-698):
   - Remove entire method implementation
   - Use default from `src/multiplexer/mod.rs:205-392`
   - Default implementation uses `send_keys()` which now works with pane targeting
   - Eliminates ~150 lines of focus navigation workarounds

2. **Keep `select_pane()` as no-op** (lines 313-317):
   - Default implementation doesn't use `select_pane()`
   - No changes needed

**Verification**: Pane creation workflows should work identically with less code.

---

### Phase 4: State Reconciliation (HIGH - Reliability)

**Objective**: Replace tab-existence validation with true pane-level validation.

**Changes to `src/multiplexer/zellij.rs`:**

1. **Rewrite `get_live_pane_info()`** (lines 768-804):
   ```rust
   fn get_live_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>> {
       let panes = Self::list_panes()?;

       // Extract numeric ID from "terminal_X"
       let numeric_id: u32 = pane_id
           .strip_prefix("terminal_")
           .and_then(|s| s.parse().ok())
           .ok_or_else(|| anyhow!("Invalid pane_id: {}", pane_id))?;

       // Find pane by ID
       let pane = match panes.iter().find(|p| p.id == numeric_id) {
           Some(p) => p,
           None => return Ok(None), // Pane doesn't exist
       };

       // Return pane info with command, title, tab name
       Ok(Some(LivePaneInfo { ... }))
   }
   ```

2. **Rewrite `get_all_live_pane_info()`** (lines 860-891):
   - Use `list_panes()` to batch query all panes
   - Build HashMap with `"terminal_{id}"` keys
   - More efficient than current client-based approach

3. **Improve `validate_agent_alive()`** (lines 806-858):
   - Primary: Check pane existence via `get_live_pane_info()`
   - Secondary: Validate command matches (agent still running)
   - Tertiary: Keep heartbeat check for performance optimization
   - Remove `cached_tabs` parameter (no longer needed)

**Verification**: Dashboard should detect dead agents even when unfocused. Kill agent processes in background panes and verify detection.

---

### Phase 5: Window Management (LOW - Optimization)

**Objective**: Use `list-tabs` for better efficiency and metadata access.

**Changes to `src/multiplexer/zellij.rs`:**

1. **Update `query_tab_names()`** (lines 77-84):
   - Mark deprecated, use `list_tabs()` internally
   - Keep for backward compatibility

2. **Update `get_all_window_names()`** (lines 268-275):
   - Use `list_tabs()` instead of `query_tab_names()`
   - Access to richer metadata for future features

3. **Add helper for future use** (new method):
   ```rust
   fn get_tab_id_by_name(name: &str) -> Result<Option<u32>>
   ```

**Verification**: Window enumeration should work identically with access to tab IDs.

---

### Phase 6: Preview Support (FUTURE - Optional)

**Status**: Deferred pending investigation

**Research needed**:
- Does `dump-screen` now support `--pane-id`?
- Check `zellij action dump-screen --help`
- If yes, can remove dashboard recursion detection (lines 390-431)

**Recommendation**: Skip for initial implementation, revisit after upstream verification.

## Implementation Sequence

1. **Phase 1** (Foundation) - 4-6 hours
   - Add pane query infrastructure
   - Update ID format methods
   - Test: Verify real pane IDs in state files

2. **Phase 2** (Pane Targeting) - 2-3 hours
   - Update send methods with `--pane-id`
   - Update capabilities
   - Test: Multi-pane command sending

3. **Phase 3** (Cleanup) - 1 hour
   - Remove `setup_panes()` override
   - Test: Pane creation workflows

4. **Phase 4** (Reconciliation) - 3-4 hours
   - Implement new validation logic
   - Test: Agent lifecycle detection

5. **Phase 5** (Optimization) - 1-2 hours
   - Switch to `list-tabs`
   - Test: Window management

**Total effort**: 11-16 hours of development + testing

## Testing Strategy

**Integration Tests**:
1. Create worktree with multiple panes
2. Send commands to unfocused panes (verify Phase 2)
3. Kill agent in background pane, verify dashboard detects it (verify Phase 4)
4. Test pane creation with different layouts (verify Phase 3)

**Regression Tests**:
1. Single-pane workflows (should work identically)
2. Tab switching (should work identically)
3. State file persistence (new pane ID format)

**Manual Verification**:
```bash
# After Phase 1+2
workmux create-workspace test-multi
# Should see terminal_0, terminal_1, etc. in state files
# Commands to unfocused panes should execute

# After Phase 4
# Kill agent process, refresh dashboard
# Should show agent as dead/missing
```

## Risk Mitigation

**State File Migration**:
- Old state files use tab names as pane IDs (e.g., `"wm-feature"`)
- New state files use numeric IDs (e.g., `"terminal_0"`)
- Migration: Old files will fail validation, dashboard will clean up automatically
- Users need to re-register agents (acceptable for major improvement)

**Zellij Version Requirements**:
- Need minimum version with `list-panes`, `list-tabs`, `--pane-id` support
- Add version check in `is_running()` to provide clear error message
- Recommend zellij 0.41.0+ (verify actual version during implementation)

**Rollback Plan**:
- All changes isolated to `zellij.rs`
- Easy to revert via git without affecting other backends
- Can temporarily set `pane_targeting: false` if critical issues arise

## Success Criteria

**Minimum viable (Phases 1-4)**:
- ✓ Real numeric pane IDs in state files
- ✓ Commands work on unfocused panes
- ✓ No focus navigation workarounds (~150 lines removed)
- ✓ Accurate agent validation for unfocused panes
- ✓ `pane_targeting: true` and `stable_pane_ids: true`

**Quality metrics**:
- Code reduction: ~150 lines removed from custom `setup_panes()`
- Zero regressions: All existing workflows work identically
- Performance: No noticeable slowdown in dashboard refresh

## Verification Plan

After implementation, verify end-to-end workflow:

1. **Create multi-pane workspace**:
   ```bash
   workmux create-workspace --worktree-path ~/test --name test-zellij
   ```

2. **Verify pane IDs**:
   - Check state files contain `"terminal_0"`, `"terminal_1"`, etc.
   - Not tab names like `"wm-test-zellij"`

3. **Test unfocused pane targeting**:
   - Create 2-pane layout
   - Focus one pane
   - Send command to other pane via dashboard
   - Verify command executes in unfocused pane

4. **Test validation**:
   - Create agent pane
   - Background pane, kill agent process with `kill <pid>`
   - Refresh dashboard
   - Verify agent shows as dead/missing

5. **Test backward compatibility**:
   - Single-pane workflows
   - Tab switching
   - Agent registration
   - Status updates

All workflows should work identically or better than before.
