---
description: Restore worktree windows after a tmux or computer crash
---

# resurrect

Restores worktree windows after a tmux or computer crash. Uses persisted agent state files to detect which worktrees had active agents before the crash, then reopens them with `--continue` to resume agent conversations.

```bash
workmux resurrect [--dry-run]
```

## Options

- `--dry-run`: Show what would be restored without actually doing it.

## How it works

1. Reads agent state files from `~/.local/state/workmux/agents/`
2. Filters to the current multiplexer backend and instance
3. Matches each state file's working directory to a git worktree in the current repo
4. Skips worktrees that are already open, no longer exist, or are the main worktree
5. Opens each matched worktree with `--continue` to resume the agent conversation
6. Cleans up consumed stale state files

## Examples

```bash
# Preview what would be restored
workmux resurrect --dry-run

# Restore all worktrees that had agents running
workmux resurrect
```

## Example output

```
  continue-flag        -> restoring
  dashboard-fix        -> skipping (already open)
  auth-refactor        -> restoring
  (2 state file(s) from other projects ignored)

✓ Restored 2 worktree(s): continue-flag, auth-refactor
```
