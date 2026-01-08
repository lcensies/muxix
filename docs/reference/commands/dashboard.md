# dashboard

Opens a TUI dashboard showing all active AI agents across all tmux sessions.

```bash
workmux dashboard
```

## Keybindings

| Key       | Action                                  |
| --------- | --------------------------------------- |
| `1`-`9`   | Quick jump to agent (closes dashboard)  |
| `d`       | View uncommitted changes (diff modal)   |
| `D`       | View branch changes vs main             |
| `p`       | Peek at agent (dashboard stays open)    |
| `s`       | Cycle sort mode                         |
| `i`       | Enter input mode (type to agent)        |
| `Ctrl+u`  | Scroll preview up                       |
| `Ctrl+d`  | Scroll preview down                     |
| `Enter`   | Go to selected agent (closes dashboard) |
| `j`/`k`   | Navigate up/down                        |
| `q`/`Esc` | Quit                                    |

### Diff modal keybindings

When viewing a diff (`d` or `D`):

| Key          | Action                              |
| ------------ | ----------------------------------- |
| `j`/`k`      | Scroll down/up                      |
| `Ctrl+d`/`u` | Page down/up                        |
| `PageDown`   | Page down                           |
| `PageUp`     | Page up                             |
| `c`          | Send commit command to agent        |
| `m`          | Trigger merge and exit dashboard    |
| `q`/`Esc`    | Close diff modal                    |

## Sort modes

Press `s` to cycle through sort modes:

- **Priority** (default): Waiting > Done > Working > Stale
- **Project**: Group by project name, then by priority within each project
- **Recency**: Most recently updated first
- **Natural**: Original tmux order (by pane creation)

Your sort preference persists in the tmux session.

See the [Dashboard guide](/guide/dashboard) for more details.
