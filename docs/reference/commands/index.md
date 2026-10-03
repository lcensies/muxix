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
| [`status`](./status)           | Query agent status and completion for worktrees |
| [`wait`](./wait)               | Wait for agents to reach a target status        |
| [`signal`](./signal)           | Emit an out-of-band agent signal                |
| [`open`](./open)               | Open a tmux window for an existing worktree     |
| [`close`](./close)             | Close a worktree's tmux window (keeps worktree) |
| [`resurrect`](./resurrect)     | Restore worktree windows after a crash          |
| [`sync-files`](./sync-files)   | Re-apply file operations to existing worktrees  |
| [`path`](./path)               | Get the filesystem path of a worktree           |
| [`dashboard`](./dashboard)     | TUI dashboard for monitoring agents             |
| [`sidebar`](./sidebar)         | Live agent status sidebar in tmux               |
| [`config edit`](./config)      | Edit the global configuration file              |
| [`init`](./init)               | Generate configuration file                     |
| [`setup`](./setup)             | Install agent hooks, skills, plugins, and MCP   |
| [`provision`](./provision)     | Sync org policy and route agents via a gateway  |
| [`profile`](./provision#workmux-profile) | Show, export, or diff the config profile |
| [`claude prune`](./claude)     | Clean up stale Claude Code entries              |
| [`completions`](./completions) | Generate shell completions                      |
| [`docs`](./docs)               | Show detailed documentation                     |
| [`update`](./update)           | Update workmux to the latest version            |
| [`last-done`](./last-done)     | Switch to the most recently completed agent     |
