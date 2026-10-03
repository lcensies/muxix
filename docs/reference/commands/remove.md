---
description: Remove worktrees, tmux windows, and branches without merging
---

# remove

Removes worktrees, tmux windows, and branches without merging (unless you keep the branches). Useful for abandoning work or cleaning up experimental branches. Supports removing multiple worktrees in a single command. Alias: `rm`

```bash
muxix remove [name]... [flags]
```

## Arguments

- `[name]...`: One or more worktree names (the directory names). Defaults to current directory name if omitted.

## Options

| Flag                | Description                                                                                                                                                                      |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--all`             | Remove all worktrees at once (except the main worktree). Prompts for confirmation unless `--force` is used. Safely skips worktrees with uncommitted changes or unmerged commits. |
| `--gone`            | Remove worktrees whose upstream remote branch has been deleted (e.g., after a PR is merged on GitHub). Automatically runs `git fetch --prune` first.                             |
| `--force, -f`       | Skip confirmation prompt and ignore uncommitted changes.                                                                                                                         |
| `--keep-branch, -k` | Remove only the worktree and tmux window while keeping the local branch.                                                                                                         |

## Examples

```bash
# Remove the current worktree (run from within the worktree)
muxix remove

# Remove a specific worktree with confirmation if unmerged
muxix remove experiment

# Remove multiple worktrees at once
muxix rm feature-a feature-b feature-c

# Remove multiple worktrees with force (no confirmation)
muxix rm -f old-work stale-branch

# Use the alias
muxix rm old-work

# Remove worktree/window but keep the branch
muxix remove --keep-branch experiment

# Force remove without prompts
muxix rm -f experiment

# Remove worktrees whose remote branches were deleted (e.g., after PR merge)
muxix rm --gone

# Force remove all gone worktrees (no confirmation)
muxix rm --gone -f

# Remove all worktrees at once
muxix rm --all
```
