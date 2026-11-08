---
description: Open a TUI dashboard showing all active AI agents
---

# dashboard

Opens a TUI dashboard showing all active AI agents across all tmux sessions.

```bash
muxix dashboard
```

## Options

- `-d, --diff`: Open the diff view directly for the current worktree's agent.
- `-P, --preview-size <10-90>`: Set preview pane size as percentage (larger = more preview, less table). Default: 60.
- `-s, --session`: Filter to only show agents in the current session.
- `-t, --tab <agents|worktrees>`: Open directly on the specified tab.

## Git stats

The `+/-` and branch columns come from the [daemon](/reference/commands/daemon)
when one is running: it already keeps a git status cache for every active
worktree, so the dashboard reads that instead of running its own `git`. The
paths on screen are also sent as a refresh hint, so a freshly opened dashboard
does not wait for the daemon's next sweep.

With no daemon — or if it exits while the dashboard is open — the dashboard
computes the statuses itself. It never waits on the daemon to render.

## Examples

```bash
# Open dashboard with default layout
muxix dashboard

# Open with smaller preview pane (40% of height)
muxix dashboard --preview-size 40

# Open diff view directly for current worktree
muxix dashboard --diff

# Show only agents in the current tmux session
muxix dashboard --session

# Open directly on the Worktrees tab
muxix dashboard --tab worktrees
```

See the [Dashboard guide](/guide/dashboard/) for keybindings and detailed documentation.
