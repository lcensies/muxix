# Zellij Dashboard Preview Footer Repetition - Fix Summary

## Problem

When running `workmux dashboard` in Zellij, the preview section shows a recursive repetition of the dashboard's footer text, with each line of the footer appearing multiple times and overlapping until it fills the entire preview area.

## Root Cause

**Zellij CLI Limitation:**
- Zellij's `zellij action dump-screen` command captures the **currently focused pane only**
- Unlike tmux (`tmux capture-pane -t <pane-id>`) or WezTerm (`wezterm cli get-text --pane-id <id>`), Zellij cannot capture arbitrary/unfocused panes
- The `pane_id` parameter in workmux's `capture_pane()` trait method is ignored by Zellij's implementation

**The Recursive Loop:**

1. Dashboard runs in Zellij and has focus (for user interaction)
2. Dashboard calls `mux.capture_pane(agent_pane_id, 200)` to preview an agent
3. Zellij's implementation ignores `agent_pane_id` and runs `dump-screen`
4. `dump-screen` captures the **dashboard itself** (the focused pane)
5. Captured content includes: agent table + previous preview + footer
6. This becomes the new preview content
7. When rendered:
   - Preview area shows the old dashboard screenshot (including old footer)
   - Real footer is rendered below the preview
   - Result: Footer appears twice
8. Next refresh cycle: Captures screen with 2 footers → creates 3 footers
9. Continues until preview is filled with repeated footer text

**Code Location:**
- `src/multiplexer/zellij.rs:326-343` - `capture_pane()` implementation
- Comment even acknowledges: "dump-screen captures entire screen, not specific pane"

## Solution: Detect Self-Capture

Add logic to detect when Zellij would capture the dashboard itself and return `None` instead.

### Implementation

**File:** `src/multiplexer/zellij.rs` (lines 326-343)

**Before:**
```rust
fn capture_pane(&self, _pane_id: &str, _lines: u16) -> Option<String> {
    // dump-screen captures entire screen, not specific pane
    // Create a temp file for output
    let temp_path = std::env::temp_dir()
        .join(format!("zellij_capture_{}", std::process::id()));
    let temp_str = temp_path.to_string_lossy();

    if Cmd::new("zellij")
        .args(&["action", "dump-screen", &temp_str])
        .run()
        .is_ok()
    {
        let content = std::fs::read_to_string(&temp_path).ok();
        let _ = std::fs::remove_file(&temp_path);
        content
    } else {
        None
    }
}
```

**After:**
```rust
fn capture_pane(&self, pane_id: &str, _lines: u16) -> Option<String> {
    // Zellij limitation: dump-screen always captures the focused pane,
    // not the pane specified by pane_id. If we try to capture while
    // the dashboard is focused, we'll capture the dashboard itself,
    // creating a recursive loop where the footer repeats infinitely.

    // Get our own pane ID to detect self-capture
    let current_pane = Self::pane_id_from_env();

    // If trying to capture ourselves, return None to avoid recursion
    if let Some(current) = current_pane {
        if pane_id == current {
            return None;
        }
    }

    // Proceed with dump-screen (will capture focused pane)
    let temp_path = std::env::temp_dir()
        .join(format!("zellij_capture_{}", std::process::id()));
    let temp_str = temp_path.to_string_lossy();

    if Cmd::new("zellij")
        .args(&["action", "dump-screen", &temp_str])
        .run()
        .is_ok()
    {
        let content = std::fs::read_to_string(&temp_path).ok();
        let _ = std::fs::remove_file(&temp_path);
        content
    } else {
        None
    }
}
```

**Key Changes:**
1. Remove `_` prefix from `pane_id` parameter (we now use it)
2. Get current pane ID using `Self::pane_id_from_env()`
3. Check if requested `pane_id` matches current pane
4. Return `None` if trying to capture self (prevents recursion)
5. Updated comment to explain the Zellij limitation

## Behavior After Fix

### When Dashboard is Focused in Zellij
- Preview shows: `"(pane not available)"`
- No recursive footer repetition
- Dashboard UI remains clean and stable
- Users can still interact with table, filters, sorting

### When Agent Pane is Focused
- `dump-screen` captures the agent pane (not dashboard)
- Preview works and shows agent output
- (Though this requires manually switching focus away from dashboard)

### Other Multiplexers (tmux/WezTerm)
- No changes to behavior
- Preview continues to work normally
- Can capture arbitrary panes regardless of focus

## Known Limitation

**Preview functionality is limited in Zellij dashboard** due to Zellij CLI constraints:
- Dashboard must stay focused for user interaction (keyboard navigation, input mode)
- When focused, it cannot capture other panes (only itself)
- This is a fundamental Zellij CLI limitation, not a workmux bug

**Workaround for users:**
- Focus the agent tab directly in Zellij to view its output
- Use `[Enter]` key in dashboard to jump to the selected agent's tab

## Why Not Use Alternative Approaches?

### Alternative 1: Content Filtering
- Strip dashboard UI patterns (footer, table) from captured content
- **Rejected because:**
  - Fragile pattern matching (breaks if footer text changes)
  - Risk of filtering legitimate agent output
  - Doesn't solve core limitation (still can't capture unfocused panes)
  - Added complexity for minimal benefit

### Alternative 2: Focus Target Pane Before Capture
- Focus the agent pane, run dump-screen, focus back to dashboard
- **Rejected because:**
  - Extremely disruptive UX (visible flickering/jumping)
  - Race conditions with user input
  - Breaks keyboard navigation
  - Accessibility concerns

### Alternative 3: Disable All Preview in Zellij
- Make `capture_pane()` always return `None` for Zellij
- **Rejected because:**
  - Too aggressive (blocks even valid captures)
  - Detecting self-capture is simple and more precise

## Testing

1. **Before fix:** Run dashboard in Zellij, observe footer repetition
2. **After fix:** Preview shows "(pane not available)", no recursion
3. **Verify tmux:** Preview still works in tmux
4. **Verify WezTerm:** Preview still works in WezTerm

## Additional Dashboard Behavior Decisions

### Dashboard Exit on Jump Behavior (Commit e0db785)

**Decision: Remove `should_exit_on_jump()` trait method**

**Previous Behavior:**
- Trait method `should_exit_on_jump()` in `src/multiplexer/mod.rs`
- Default: `true` (tmux/WezTerm exit dashboard after jumping to agent)
- Zellij override: `false` (keep dashboard open after jumping)

**Change Made:**
- Removed `should_exit_on_jump()` trait method entirely
- All backends now consistently close dashboard when jumping to an agent (1-9 keys or Enter)
- Simplified `jump_to_selected()` in `src/command/dashboard/app.rs:424-434`

**Rationale:**
- Consistent behavior across all multiplexers
- Simpler code (no need for per-backend customization)
- After jumping to an agent, users can easily return to dashboard if needed

**Files Modified:**
- `src/multiplexer/mod.rs`: Removed `should_exit_on_jump()` trait method
- `src/multiplexer/zellij.rs`: Removed `should_exit_on_jump()` implementation
- `src/command/dashboard/app.rs`: Simplified jump logic to always set `should_jump = true`

## Dashboard Title Handling Decision

### Initial Attempt: Command-Based Title Fallback (Commit e0db785)

**Problem:**
- Zellij doesn't expose pane titles via CLI (unlike tmux's `#{pane_title}`)
- Dashboard showed empty titles for all Zellij agents

**Solution Attempted:**
1. Modified `get_live_pane_info()` in `src/multiplexer/zellij.rs:719-733` to use running command as title fallback
2. Modified `to_agent_pane()` in `src/state/types.rs:116-125` to use command field as title when pane_title is None
3. This filled the title column with command names (e.g., "claude")

**Code Changes:**
- `src/multiplexer/zellij.rs`: Added logic to set `title: Some(current_command)` instead of `title: None`
- `src/state/types.rs`: Added fallback logic to use `self.command` when `self.pane_title` is None

### Revert Decision (Commit dc0c26f)

**After testing, the title fallback was reverted because:**
- The command-based titles broke the dashboard display
- Testing showed issues with how titles were rendered

**Final Behavior:**
- Dashboard shows empty titles for Zellij panes (title column is blank)
- This is acceptable because:
  - Other columns (session, window, path, status) provide enough context
  - Users can still identify agents by their path and status
  - Empty titles are better than broken display

**Reverted Changes:**
- `src/multiplexer/zellij.rs`: Removed command-based title fallback, back to `title: None`
- `src/state/types.rs`: Removed `or_else()` logic, back to direct `pane_title: self.pane_title.clone()`

**Conclusion:**
For Zellij, we accept the limitation that pane titles cannot be displayed in the dashboard due to CLI constraints. This is consistent with other Zellij limitations (like preview functionality).

## Related Issues

- [zellij-dashboard-persistence-issue.md](./zellij-dashboard-persistence-issue.md) - Agent persistence fix
- [ZELLIJ_FIX_IMPLEMENTATION.md](./ZELLIJ_FIX_IMPLEMENTATION.md) - Trait-based validation implementation

## Files Modified

1. `src/multiplexer/zellij.rs` - `capture_pane()` method (lines 326-343)
2. `src/multiplexer/zellij.rs` - `get_live_pane_info()` method (title handling - reverted)
3. `src/state/types.rs` - `to_agent_pane()` method (title fallback logic - reverted)
