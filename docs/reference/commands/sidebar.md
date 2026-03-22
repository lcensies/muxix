---
description: Toggle a compact agent status sidebar across all tmux windows
---

# sidebar

Toggles a compact agent status sidebar on the left side of all tmux windows.
Shows all active agents across all sessions and projects with live status
updates.

```bash
workmux sidebar            # Toggle sidebar on/off
workmux sidebar --width 40 # Custom width (default: 30)
```

## What it shows

Each agent row displays:

- Status icon (working/waiting/done with spinner animation)
- Project and worktree name (e.g. `myproject/fix-bug`)
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

When enabled, the sidebar creates a narrow tmux pane on the left side of every
existing window using a full-height split. Each pane runs a lightweight TUI that
polls agent state every 2 seconds. A tmux hook (`after-new-window`) ensures
newly created windows also get a sidebar automatically.

Running `workmux sidebar` again disables the sidebar globally, killing all
sidebar panes and removing the hook.

## Limitations

- tmux only (other backends are not supported yet)

## Example tmux binding

```bash
bind C-t run-shell "workmux sidebar"
```
