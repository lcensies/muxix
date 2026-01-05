# Status popup

When running multiple AI agents in parallel, it's helpful to have a centralized view of what each agent is doing. The status popup provides a TUI for monitoring all active agents across all tmux sessions.

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/status-popup.webp" alt="workmux status popup" style="border-radius: 4px;">
</div>

## Setup

Add this binding to your `~/.tmux.conf`:

```bash
bind C-s display-popup -h 15 -w 100 -E "workmux status"
```

Then press `prefix + Ctrl-s` to open the dashboard as an overlay. Feel free to adjust the keybinding and popup dimensions (`-h` and `-w`) as needed.

The popup shows all tmux panes that have agent status set (via the [status tracking](/guide/agents#status-tracking) hooks).

## Keybindings

| Key     | Action                              |
| ------- | ----------------------------------- |
| `1`-`9` | Quick jump to agent (closes popup)  |
| `p`     | Peek at agent (popup stays open)    |
| `Enter` | Go to selected agent (closes popup) |
| `j`/`k` | Navigate up/down                    |
| `q`     | Quit                                |

## Columns

- **#**: Quick jump key (1-9)
- **Project**: Project name (from `__worktrees` path or directory name)
- **Agent**: Worktree/window name
- **Status**: Agent status icon (🤖 working, 💬 waiting, ✅ done, or "stale")
- **Time**: Time since last status change
- **Title**: Claude Code session title (auto-generated summary)
