---
description: Complete reference for all workmux commands
---

# CLI reference

## Commands overview

| Command                        | Description                                     |
| ------------------------------ | ----------------------------------------------- |
| [`add`](./add)                 | Create a new worktree and tmux window           |
| [`merge`](./merge)             | Merge a branch and clean up everything          |
| [`remove`](./remove)           | Remove worktrees without merging                |
| [`rename`](./rename)           | Rename a worktree, its tmux window, and branch  |
| [`list`](./list)               | List all worktrees with status                  |
| [`open`](./open)               | Open a tmux window for an existing worktree     |
| [`close`](./close)             | Close a worktree's tmux window (keeps worktree) |
| [`resurrect`](./resurrect)     | Restore worktree windows after a crash          |
| [`sync-files`](./sync-files)   | Re-apply file operations to existing worktrees  |
| [`path`](./path)               | Get the filesystem path of a worktree           |
| [`dashboard`](./dashboard)     | TUI dashboard for monitoring agents             |
| [`sidebar`](./sidebar)         | Live agent status sidebar in tmux               |
| [`config edit`](./config)      | Edit the global configuration file              |
| [`init`](./init)               | Generate configuration file                     |
| [`claude prune`](./claude)     | Clean up stale Claude Code entries              |
| [`completions`](./completions) | Generate shell completions                      |
| [`docs`](./docs)               | Show detailed documentation                     |
| [`update`](./update)           | Update workmux to the latest version            |
| [`last-done`](./last-done)     | Switch to the most recently completed agent     |
