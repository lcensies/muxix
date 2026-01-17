---
description: A TUI for monitoring agents, reviewing changes, staging hunks, and sending commands
---

# Dashboard

When running agents in multiple worktrees across many projects, it's helpful to have a centralized view of what each agent is doing. The dashboard provides a TUI for monitoring agents, reviewing their changes, staging hunks, and sending commands.

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/dashboard.webp" alt="workmux dashboard" style="border-radius: 4px;">
</div>

## Setup

Add this binding to your `~/.tmux.conf`:

```bash
bind C-s display-popup -h 30 -w 100 -E "workmux dashboard"
```

Then press `prefix + Ctrl-s` to open the dashboard as a tmux popup. Feel free to adjust the keybinding and popup dimensions (`-h` and `-w`) as needed.

::: tip Quick access
Consider binding the dashboard to a key you can press without the tmux prefix, such as `Cmd+E` or `Ctrl+E` in your terminal emulator. This makes it easy to check on your agents at any time.
:::

::: warning Prerequisites
This feature requires [status tracking hooks](/guide/status-tracking) to be configured. Without them, no agents will appear in the dashboard.
:::

See [command reference](/reference/commands/dashboard) for CLI options.

## Keybindings

| Key       | Action                                  |
| --------- | --------------------------------------- |
| `1`-`9`   | Quick jump to agent (closes dashboard)  |
| `d`       | View diff (opens WIP view)              |
| `p`       | Peek at agent (dashboard stays open)    |
| `s`       | Cycle sort mode                         |
| `f`       | Toggle stale filter (show/hide stale)   |
| `i`       | Enter input mode (type to agent)        |
| `Ctrl+u`  | Scroll preview up                       |
| `Ctrl+d`  | Scroll preview down                     |
| `+`/`-`   | Resize preview pane                     |
| `Enter`   | Go to selected agent (closes dashboard) |
| `j`/`k`   | Navigate up/down                        |
| `q`/`Esc` | Quit                                    |
| `Ctrl+c`  | Quit (works from any view)              |

## Columns

- **#**: Quick jump key (1-9)
- **Project**: Project name (from `__worktrees` path or directory name)
- **Agent**: Worktree/window name
- **Git**: Diff stats showing branch changes (dim) and uncommitted changes (bright)
- **Status**: Agent status icon (🤖 working, 💬 waiting, ✅ done, or "stale")
- **Time**: Time since last status change
- **Title**: Claude Code session title (auto-generated summary)

## Live preview

The bottom half of the dashboard shows a live preview of the selected agent's terminal output. The preview auto-scrolls to show the latest output, but you can scroll through history with `Ctrl+u`/`Ctrl+d`.

## Input mode

Press `i` to enter input mode, which forwards your keystrokes directly to the selected agent's pane. This lets you respond to agent prompts without leaving the dashboard. Press `Esc` to exit input mode and return to normal navigation.

## Sort modes

Press `s` to cycle through sort modes:

- **Priority** (default): Waiting > Done > Working > Stale
- **Project**: Group by project name, then by priority within each project
- **Recency**: Most recently updated first
- **Natural**: Original tmux order (by pane creation)

Your sort preference persists in the tmux session.

## Stale filter

Press `f` to toggle between showing all agents or hiding stale ones. The filter state persists across dashboard sessions within the same tmux server.
