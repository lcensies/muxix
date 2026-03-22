---
description: Toggle a compact agent status sidebar in the current tmux window
---

# sidebar

Toggles a compact agent status sidebar on the left side of the current tmux
window. Shows all active agents in the current session with live status updates.

```bash
workmux sidebar            # Toggle sidebar on/off
workmux sidebar --width 40 # Custom width (default: 30)
```

## What it shows

Each agent row displays:

- Status icon (working/waiting/done with spinner animation)
- Worktree name (truncated to fit)
- Elapsed time since last status change

## Keybindings

| Key     | Action             |
| ------- | ------------------ |
| `j`/`k` | Navigate up/down   |
| `Enter` | Jump to agent pane |
| `g`/`G` | Jump to first/last |
| `q`     | Quit sidebar       |

## Options

| Option          | Description                          |
| --------------- | ------------------------------------ |
| `-w`, `--width` | Width of the sidebar pane in columns |

## How it works

The sidebar creates a narrow tmux pane on the left side of the current window
using a full-height split. It runs a lightweight TUI that polls agent state
every 2 seconds and renders a compact list.

The sidebar pane is tagged with a tmux pane option (`@workmux_role`), so
running `workmux sidebar` again in the same window will close the existing
sidebar instead of creating a new one.

## Limitations

- tmux only (other backends are not supported yet)
- Per-window (tmux panes are bound to their window)
- Session-scoped (only shows agents from the current tmux session)

## Example tmux binding

```bash
bind C-t run-shell "workmux sidebar"
```
