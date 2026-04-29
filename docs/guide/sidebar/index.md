---
description: A persistent agent status sidebar for tmux windows
---

# Sidebar

The sidebar provides an always-visible agent overview pinned to the left edge of
every tmux window. Unlike the dashboard (which is a full-screen TUI you open on
demand), the sidebar stays on screen while you work.

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/sidebar.webp" alt="workmux sidebar" style="border-radius: 4px;">
</div>

## Setup

::: warning Prerequisites
The sidebar requires [status tracking hooks](/guide/status-tracking) to be
configured and tmux as the backend.
:::

Toggle the sidebar with:

```bash
workmux sidebar            # All sessions (default)
workmux sidebar --session  # Current session only
```

By default, the sidebar appears in all existing and newly created tmux windows
across all sessions. Use `--session` to scope it to the current session only,
leaving other sessions untouched. Running the command again disables it.

Optionally, add a tmux binding for quick access:

```bash
bind C-t run-shell "workmux sidebar"
```

## What it shows

Each agent is displayed as a tile showing:

- Status icon with spinner animation (working/waiting/done)
- Worktree name and elapsed time since last status change
- Project name and git diff stats (committed + uncommitted lines)
- Agent task description

The exact layout is fully customizable via [templates](./templates).

## Configuration

Configure the sidebar in your global `~/.config/workmux/config.yaml` or project
`.workmux.yaml`:

```yaml
sidebar:
  # Width: absolute columns or percentage of terminal width
  width: 40 # absolute columns
  # width: "15%"  # percentage of terminal width

  # Layout mode: "compact" or "tiles" (default)
  layout: tiles
```

Width defaults to 10% of terminal width, clamped between 25 and 50 columns.
When set explicitly, the clamp is removed (minimum 10 columns).

## Layout modes

The sidebar supports two layout modes, toggled with `v`:

- **Tiles** (default): variable-height cards with status stripe
- **Compact**: single line per agent

Your preference is persisted across tmux restarts.

## Mouse support

Click an agent tile to jump to its pane, or scroll to navigate the list. Requires
`set -g mouse on` in your `~/.tmux.conf`.

## Keybindings

| Key     | Action                   |
| ------- | ------------------------ |
| `j`/`k` | Navigate up/down         |
| `Enter` | Jump to agent pane       |
| `g`/`G` | Jump to first/last       |
| `v`     | Toggle layout mode       |
| `z`     | Toggle sleeping on agent |
| `q`     | Quit sidebar             |

### Sleeping agents

Press `z` to manually mark an agent as sleeping. Sleeping agents show a 💤 icon
with dimmed colors and are pushed to the bottom of the list, regardless of their
actual status. This is useful for temporarily deprioritizing agents you don't
need to monitor. Press `z` again to wake them up.

## Agent navigation hotkeys

You can switch between agents from any tmux pane using subcommands. These work
in the same order shown in the sidebar:

| Command                    | Action                               |
| -------------------------- | ------------------------------------ |
| `workmux sidebar next`     | Switch to the next agent (wraps)     |
| `workmux sidebar prev`     | Switch to the previous agent (wraps) |
| `workmux sidebar jump <N>` | Jump to the Nth agent (1-indexed)    |

### Example tmux keybindings

```bash
# Alt+j / Alt+k to cycle agents (no prefix needed)
bind -n M-j run-shell "workmux sidebar next"
bind -n M-k run-shell "workmux sidebar prev"

# Alt+1..9 to jump directly
bind -n M-1 run-shell "workmux sidebar jump 1"
bind -n M-2 run-shell "workmux sidebar jump 2"
bind -n M-3 run-shell "workmux sidebar jump 3"
# ...
```
