# dashboard

Opens a TUI dashboard showing all active AI agents across all tmux sessions.

```bash
workmux dashboard
```

## Keybindings

| Key       | Action                                  |
| --------- | --------------------------------------- |
| `1`-`9`   | Quick jump to agent (closes dashboard)  |
| `d`       | View diff (opens WIP view)              |
| `p`       | Peek at agent (dashboard stays open)    |
| `s`       | Cycle sort mode                         |
| `i`       | Enter input mode (type to agent)        |
| `Ctrl+u`  | Scroll preview up                       |
| `Ctrl+d`  | Scroll preview down                     |
| `Enter`   | Go to selected agent (closes dashboard) |
| `j`/`k`   | Navigate up/down                        |
| `q`/`Esc` | Quit                                    |

### Diff view keybindings

When viewing a diff (`d`):

| Key       | Action                            |
| --------- | --------------------------------- |
| `d`       | Toggle WIP / review               |
| `a`       | Enter patch mode (WIP only)       |
| `j`/`k`   | Scroll down/up                    |
| `Ctrl+d`  | Page down                         |
| `Ctrl+u`  | Page up                           |
| `c`       | Send commit command to agent      |
| `m`       | Trigger merge and exit dashboard  |
| `q`/`Esc` | Close diff view                   |

The footer shows which diff type is active: **WIP** (uncommitted changes) or **review** (branch vs main). Press `d` to toggle between them.

### Patch mode keybindings

Patch mode (`a` from WIP diff) allows staging individual hunks like `git add -p`:

| Key       | Action                            |
| --------- | --------------------------------- |
| `y`       | Stage current hunk                |
| `n`       | Skip current hunk                 |
| `u`       | Undo last staged hunk             |
| `s`       | Split hunk (if splittable)        |
| `j`/`k`   | Navigate to next/previous hunk    |
| `q`/`Esc` | Exit patch mode                   |

Staging a hunk adds it to the git index. After staging, the diff refreshes to show remaining unstaged changes. Pressing `s` splits a hunk into smaller hunks if there are context lines between changes. Use `u` to undo the last staged hunk if you made a mistake.

## Sort modes

Press `s` to cycle through sort modes:

- **Priority** (default): Waiting > Done > Working > Stale
- **Project**: Group by project name, then by priority within each project
- **Recency**: Most recently updated first
- **Natural**: Original tmux order (by pane creation)

Your sort preference persists in the tmux session.

See the [Dashboard guide](/guide/dashboard) for more details.
