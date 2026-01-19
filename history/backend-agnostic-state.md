# Backend-Agnostic State Storage

This document outlines the plan to decouple workmux's state storage from tmux,
enabling support for alternative terminal multiplexers like WezTerm and Zellij.

## Problem Statement

Currently, workmux stores all agent state in tmux server-side user options
(`@workmux_*` variables). This creates several issues:

1. **Tight coupling**: State storage is intertwined with tmux-specific APIs
2. **No portability**: Other multiplexers (WezTerm, Zellij) lack equivalent
   server-side state
3. **Fragility**: State is lost if tmux server restarts
4. **Brittle exit detection**: Comparing `pane_current_command` fails with
   wrappers, SSH, or re-exec

### Current State Locations

| State             | Scope  | tmux Variable             | Purpose                    |
| ----------------- | ------ | ------------------------- | -------------------------- |
| Status icon       | Pane   | `@workmux_pane_status`    | Dashboard tracking         |
| Status timestamp  | Pane   | `@workmux_pane_status_ts` | Recency sorting, staleness |
| Monitored command | Pane   | `@workmux_pane_command`   | Exit detection             |
| Status icon       | Window | `@workmux_status`         | Status bar display         |
| Status timestamp  | Window | `@workmux_status_ts`      | Auto-clear logic           |
| Sort mode         | Global | `@workmux_sort_mode`      | Dashboard preference       |
| Hide stale filter | Global | `@workmux_hide_stale`     | Dashboard preference       |
| Preview size      | Global | `@workmux_preview_size`   | Dashboard preference       |

## Solution: Filesystem-Based State Store

Decouple state storage from the multiplexer. Use the filesystem as the source of
truth; the multiplexer becomes a view layer only.

### Design Principles

1. **Multiplexer as view layer**: Only use tmux/WezTerm/Zellij for listing
   panes, sending keys, and updating UI (status bar)
2. **Filesystem as database**: JSON files in XDG state directory
3. **Lock-free concurrency**: One file per pane prevents write conflicts
4. **PID-based validation**: Detect pane ID recycling reliably
5. **Lazy cleanup**: No background daemon; reconciliation happens on-demand
6. **Explicit exit signaling**: Agents call `set-window-status clear` on exit

### Directory Structure

```
$XDG_STATE_HOME/workmux/           # ~/.local/state/workmux/
├── settings.json                   # Global dashboard settings
└── agents/
    ├── tmux__default__%1.json     # {backend}__{instance}__{pane_id}.json
    ├── tmux__default__%5.json
    └── wezterm__main__3.json
```

File naming uses double underscores as delimiters since pane IDs may contain
single underscores.

## Data Structures

### Agent State (`agents/*.json`)

```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct AgentState {
    /// Composite identifier for the pane
    pub pane_key: PaneKey,

    /// Working directory of the agent
    pub workdir: PathBuf,

    /// Current status icon (e.g., "🤖", "💬", "✅")
    pub status: Option<String>,

    /// Unix timestamp when status was last set
    pub status_ts: Option<u64>,

    /// Pane title (set by Claude Code to show session summary)
    pub pane_title: Option<String>,

    /// PID of the pane's shell process (for pane ID recycling detection)
    pub pane_pid: u32,

    /// Foreground command when status was set (for agent exit detection)
    /// If this changes (e.g., "node" → "zsh"), the agent has exited.
    pub command: String,

    /// Unix timestamp of last state update (status change)
    /// Note: This is NOT a heartbeat - it's only updated when status changes.
    /// Used for staleness detection and recency sorting.
    pub updated_ts: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Hash)]
pub struct PaneKey {
    /// Backend type: "tmux", "wezterm", "zellij"
    pub backend: String,

    /// Backend instance identifier (e.g., tmux socket path, wezterm mux ID)
    pub instance: String,

    /// Pane identifier within the backend
    pub pane_id: String,
}
```

### Global Settings (`settings.json`)

```rust
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct GlobalSettings {
    pub sort_mode: String,           // "priority", "project", "recency", "natural"
    pub hide_stale: bool,
    pub preview_size: Option<u8>,    // 10-90 percentage
}
```

## Pane Identification Strategy

### Composite Key

Each pane is uniquely identified by `(backend, instance, pane_id)`:

| Backend | Instance                 | Pane ID     |
| ------- | ------------------------ | ----------- |
| tmux    | Socket path or "default" | `%1`, `%42` |
| WezTerm | Mux domain ID            | Tab ID      |
| Zellij  | Session name             | Pane ID     |

### PID Validation

Pane IDs can be recycled (e.g., tmux `%1` closes, new pane gets `%1`). To detect
this:

1. Store `pane_pid` when creating agent state
2. On dashboard poll, fetch live pane's PID from multiplexer
3. If PIDs don't match, the state file is stale - delete it

```rust
// Reconciliation pseudocode (runs when dashboard opens)
for state_file in read_all_agent_files() {
    let live_pane = backend.get_pane_info(&state_file.pane_key)?;

    match live_pane {
        None => {
            // Pane no longer exists in multiplexer
            delete_state_file(&state_file.pane_key);
        }
        Some(live) if live.pid != state_file.pane_pid => {
            // PID mismatch - pane ID was recycled by a new process
            delete_state_file(&state_file.pane_key);
        }
        Some(live) if live.current_command != state_file.command => {
            // Command changed - agent exited (e.g., "node" → "zsh")
            delete_state_file(&state_file.pane_key);
        }
        Some(live) => {
            // Valid - include in dashboard
            // Note: may still be "stale" if updated_ts is old, but agent is alive
            agents.push(state_file);
        }
    }
}
```

## Exit Detection and Cleanup

**Important**: The dashboard does not run continuously. It only opens when the
user invokes it. This means cleanup cannot rely on continuous polling or
background processes.

### Two-Layer Detection

Exit detection uses two complementary mechanisms:

| Mechanism | Detects | How |
|-----------|---------|-----|
| **Command comparison** | Agent exited within pane | Foreground command changed (e.g., `node` → `zsh`) |
| **PID validation** | Pane was closed and recycled | Stored pane PID doesn't match live pane PID |

**Why both are needed:**

- `pane_pid` is the shell's PID, not the agent's. If an agent crashes, the shell
  stays alive with the same PID. Command comparison catches this.
- If a pane is closed entirely and a new pane gets the same ID (e.g., `%1`),
  command comparison might accidentally match. PID validation catches this.

### How Cleanup Works

1. **Graceful exit**: Agent calls `workmux set-window-status clear`, which
   deletes the state file immediately
2. **Ungraceful exit** (crash, kill -9): State file persists until the next
   dashboard open
3. **Dashboard reconciliation**: When dashboard opens, it checks each state file:
   - Pane no longer exists → delete state file
   - PID mismatch (pane recycled) → delete state file
   - Command changed (agent exited) → delete state file

### State File Lifecycle

```
Agent starts
    │
    ▼
set-window-status working  ──►  State file created (stores pane_pid + command)
    │
    ▼
Agent runs (status changes)  ──►  State file updated (updated_ts refreshed)
    │
    ├─── Graceful exit ───►  set-window-status clear  ──►  State file deleted
    │
    └─── Crash/kill ───►  State file lingers
                              │
                              ▼
                         Dashboard opens
                              │
                              ▼
                         Command comparison OR PID validation fails
                              │
                              ▼
                         State file deleted
```

### No Daemon Required

This design intentionally avoids background processes:

- State files are small JSON (~200 bytes each)
- A few orphaned files between dashboard sessions are harmless
- Cleanup happens lazily but reliably on next dashboard open

### Staleness vs Exit

- **Stale**: Agent is still running but hasn't changed status in N minutes
  (based on `updated_ts`)
- **Exited**: Pane gone, PID mismatch, or command changed back to shell

The dashboard can filter/dim stale agents, but only removes state files for
truly exited agents.

## Multiplexer Trait

Abstract multiplexer operations behind a trait:

```rust
pub trait Multiplexer {
    /// Get information about all panes in this backend instance
    fn list_panes(&self) -> Result<Vec<LivePaneInfo>>;

    /// Get the backend instance identifier
    fn instance_id(&self) -> &str;

    /// Get the backend name ("tmux", "wezterm", "zellij")
    fn backend_name(&self) -> &str;

    /// Update the UI to show agent status (status bar, tab title, etc.)
    /// This is optional - some backends may not support it
    fn update_status_display(&self, pane_id: &str, icon: &str) -> Result<()>;

    /// Clear the UI status display
    fn clear_status_display(&self, pane_id: &str) -> Result<()>;

    /// Check if a specific pane exists and get its info
    fn get_pane_info(&self, pane_id: &str) -> Result<Option<LivePaneInfo>>;

    /// Send keys to a pane
    fn send_keys(&self, pane_id: &str, keys: &str) -> Result<()>;

    /// Capture pane content
    fn capture_pane(&self, pane_id: &str, lines: u16) -> Result<Option<String>>;

    /// Switch focus to a pane
    fn switch_to_pane(&self, pane_id: &str) -> Result<()>;
}

pub struct LivePaneInfo {
    pub pane_id: String,
    pub pid: u32,
    pub current_command: String,  // Used for agent exit detection
    pub working_dir: PathBuf,
    pub title: Option<String>,
    pub session: Option<String>,
    pub window: Option<String>,
}
```

## State Store API

```rust
pub struct StateStore {
    base_path: PathBuf,  // ~/.local/state/workmux
}

impl StateStore {
    /// Create or update agent state
    pub fn upsert_agent(&self, state: &AgentState) -> Result<()>;

    /// Read agent state by pane key
    pub fn get_agent(&self, key: &PaneKey) -> Result<Option<AgentState>>;

    /// List all agent states (for reconciliation)
    pub fn list_all_agents(&self) -> Result<Vec<AgentState>>;

    /// Delete agent state
    pub fn delete_agent(&self, key: &PaneKey) -> Result<()>;

    /// Load global settings
    pub fn load_settings(&self) -> Result<GlobalSettings>;

    /// Save global settings
    pub fn save_settings(&self, settings: &GlobalSettings) -> Result<()>;
}
```

### Implementation Notes

**Atomic writes**: Use write-to-temp-then-rename for crash safety. If a process
crashes mid-write, a partial JSON file could cause parse errors on next read.

```rust
fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;  // Atomic on POSIX
    Ok(())
}
```

This is a robustness improvement, not a concurrency fix - each pane writes to
its own file, so concurrent writes to the same file don't occur. The failure
mode (corrupted JSON on rare crash) is recoverable: delete the state file and
the agent recreates it on next status update.

**Tolerant reads**: When reading, handle parse errors gracefully by treating
corrupted files as missing (log warning, delete file, continue).

## Component Interactions

```
┌─────────────────────────────────────────────────────────────────┐
│                        workmux CLI                               │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐       │
│  │  Dashboard   │    │ set-window-  │    │    Agent     │       │
│  │   Command    │    │   status     │    │   Launch     │       │
│  └──────┬───────┘    └──────┬───────┘    └──────┬───────┘       │
│         │                   │                   │                │
│         ▼                   ▼                   ▼                │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                     State Store                          │    │
│  │              ~/.local/state/workmux/                     │    │
│  └─────────────────────────────────────────────────────────┘    │
│         │                   │                   │                │
│         ▼                   ▼                   ▼                │
│  ┌─────────────────────────────────────────────────────────┐    │
│  │                  Multiplexer Trait                       │    │
│  └─────────────────────────────────────────────────────────┘    │
│         │                   │                   │                │
│         ▼                   ▼                   ▼                │
│  ┌────────────┐    ┌────────────┐    ┌────────────┐             │
│  │    Tmux    │    │  WezTerm   │    │   Zellij   │             │
│  │  Backend   │    │  Backend   │    │  Backend   │             │
│  └────────────┘    └────────────┘    └────────────┘             │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

## Migration Strategy

Direct cutover - no phased migration needed. State is ephemeral (agent status),
so the risk of a direct switch is low.

### Steps

1. Add `StateStore` module with JSON file operations
2. Replace tmux pane option reads/writes with state file reads/writes
3. Replace tmux global reads/writes with `settings.json`
4. Keep tmux window options for status bar display only (view layer)
5. Extract tmux operations into `TmuxBackend` implementing `Multiplexer`
6. Add backend detection logic

## File Changes Summary

### New Files

| File                     | Purpose                                   |
| ------------------------ | ----------------------------------------- |
| `src/state/mod.rs`       | State store module                        |
| `src/state/store.rs`     | `StateStore` implementation               |
| `src/state/types.rs`     | `AgentState`, `PaneKey`, `GlobalSettings` |
| `src/backend/mod.rs`     | Backend module, trait definition          |
| `src/backend/tmux.rs`    | `TmuxBackend` implementing `Multiplexer`  |
| `src/backend/wezterm.rs` | `WezTermBackend` (future)                 |
| `src/backend/zellij.rs`  | `ZellijBackend` (future)                  |

### Modified Files

| File                                | Changes                                        |
| ----------------------------------- | ---------------------------------------------- |
| `src/tmux.rs`                       | Extract into `backend/tmux.rs`, keep utilities |
| `src/command/set_window_status.rs`  | Write to `StateStore`, update UI via backend   |
| `src/command/dashboard/settings.rs` | Use `StateStore` instead of tmux globals       |
| `src/command/dashboard/sort.rs`     | Use `StateStore` instead of tmux globals       |
| `src/command/dashboard/app.rs`      | Use `StateStore` for agent list                |

## Testing Strategy

1. **Unit tests**: `StateStore` operations (read, write, delete, list)
2. **Unit tests**: Reconciliation logic (PID mismatch, missing pane)
3. **Integration tests**: Backend trait implementations
4. **Migration tests**: Verify dual-write produces consistent state

## Open Questions

1. **Settings scope**: Should dashboard settings be global or
   per-backend-instance?
   - Recommendation: Global (user preference, not backend-specific)

2. **Backend detection**: How to detect which backend is active?
   - Check `$TMUX`, `$WEZTERM_PANE`, `$ZELLIJ` environment variables

3. **Concurrent tmux servers**: How to handle multiple tmux servers?
   - Use socket path as instance identifier
   - Default to "default" for standard socket

## References

- [GitHub Discussion #36](https://github.com/raine/workmux/discussions/36):
  Original WezTerm support proposal
- [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html)
