# dashboard

Opens a TUI dashboard showing all active AI agents across all tmux sessions.

```bash
workmux dashboard
```

## Keybindings

| Key       | Action                                  |
| --------- | --------------------------------------- |
| `1`-`9`   | Quick jump to agent (closes dashboard)  |
| `p`       | Peek at agent (dashboard stays open)    |
| `s`       | Cycle sort mode                         |
| `i`       | Enter input mode (type to agent)        |
| `Ctrl+u`  | Scroll preview up                       |
| `Ctrl+d`  | Scroll preview down                     |
| `Enter`   | Go to selected agent (closes dashboard) |
| `j`/`k`   | Navigate up/down                        |
| `q`/`Esc` | Quit                                    |

## Sort modes

Press `s` to cycle through sort modes:

- **Priority** (default): Waiting > Done > Working > Stale
- **Project**: Group by project name, then by priority within each project
- **Recency**: Most recently updated first
- **Natural**: Original tmux order (by pane creation)

Your sort preference persists in the tmux session.

See the [Dashboard guide](/guide/dashboard) for more details.
