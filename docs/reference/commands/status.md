# status

Opens a TUI dashboard showing all active AI agents across all tmux sessions.

```bash
workmux status
```

## Keybindings

| Key     | Action                              |
| ------- | ----------------------------------- |
| `1`-`9` | Quick jump to agent (closes popup)  |
| `p`     | Peek at agent (popup stays open)    |
| `s`     | Cycle sort mode                     |
| `Enter` | Go to selected agent (closes popup) |
| `j`/`k` | Navigate up/down                    |
| `q`     | Quit                                |

## Sort modes

Press `s` to cycle through sort modes:

- **Priority** (default): Waiting > Done > Working > Stale
- **Project**: Group by project name, then by priority within each project
- **Recency**: Most recently updated first
- **Natural**: Original tmux order (by pane creation)

Your sort preference persists in the tmux session.

See the [Status popup guide](/guide/status-popup) for more details.
