---
description: A persistent agent status sidebar for tmux windows
---

# Sidebar

The sidebar provides an always-visible agent overview pinned to the left edge of
every tmux window. Unlike the dashboard (which is a full-screen TUI you open on
demand), the sidebar stays on screen while you work.

## Setup

::: warning Prerequisites
The sidebar requires [status tracking hooks](/guide/status-tracking) to be
configured and tmux as the backend.
:::

Add this binding to your `~/.tmux.conf`:

```bash
bind C-t run-shell "workmux sidebar"
```

Then press `prefix + Ctrl-t` to toggle the sidebar on or off.

## Usage

```bash
workmux sidebar            # Toggle sidebar on/off
```

The sidebar automatically appears in all existing and newly created tmux
windows. Running the command again disables it globally.

## What it shows

Each agent is displayed as a tile showing:

- Status icon with spinner animation (working/waiting/done)
- Project and worktree name (e.g. `myproject/fix-bug`)
- Elapsed time since last status change

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

## Keybindings

| Key     | Action             |
| ------- | ------------------ |
| `j`/`k` | Navigate up/down   |
| `Enter` | Jump to agent pane |
| `g`/`G` | Jump to first/last |
| `v`     | Toggle layout mode |
| `q`     | Quit sidebar       |

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

## How it works

A background daemon polls tmux state every 2 seconds and pushes snapshots to
each sidebar pane over a Unix socket. The sidebar creates a narrow tmux pane on
the left side of every window using a full-height split. A tmux hook
(`after-new-window`) ensures newly created windows also get a sidebar
automatically.
