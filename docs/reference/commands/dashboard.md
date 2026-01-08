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
| `f`       | Toggle stale filter (show/hide stale)   |
| `i`       | Enter input mode (type to agent)        |
| `Ctrl+u`  | Scroll preview up                       |
| `Ctrl+d`  | Scroll preview down                     |
| `Enter`   | Go to selected agent (closes dashboard) |
| `j`/`k`   | Navigate up/down                        |
| `q`/`Esc` | Quit                                    |
| `Ctrl+c`  | Quit (works from any view)              |

### Stale filter

Press `f` to toggle between showing all agents or hiding stale ones. The filter state persists across dashboard sessions within the same tmux server.

## Diff view

Press `d` to view the diff for the selected agent. The diff view has two modes:

- **WIP** - Shows uncommitted changes (`git diff HEAD`)
- **review** - Shows all changes on the branch vs main (`git diff main...HEAD`)

Press `Tab` while in diff view to toggle between modes. The footer displays which mode is active along with diff statistics showing lines added (+) and removed (-).

If there are no changes to show, a message is displayed instead:
- WIP mode: "No uncommitted changes"
- Review mode: "No commits on this branch yet"

### Diff view keybindings

| Key       | Action                            |
| --------- | --------------------------------- |
| `Tab`     | Toggle WIP / review               |
| `a`       | Enter patch mode (WIP only)       |
| `j`/`k`   | Scroll down/up                    |
| `Ctrl+d`  | Page down                         |
| `Ctrl+u`  | Page up                           |
| `c`       | Send commit command to agent      |
| `m`       | Trigger merge and exit dashboard  |
| `q`/`Esc` | Close diff view                   |
| `Ctrl+c`  | Quit dashboard                    |

## Patch mode

Patch mode (`a` from WIP diff) allows staging individual hunks like `git add -p`. This is useful for selectively staging parts of an agent's work.

When [delta](https://github.com/dandavison/delta) is installed, hunks are rendered with syntax highlighting for better readability.

### Patch mode keybindings

| Key       | Action                            |
| --------- | --------------------------------- |
| `y`       | Stage current hunk                |
| `n`       | Skip current hunk                 |
| `u`       | Undo last staged hunk             |
| `s`       | Split hunk (if splittable)        |
| `c`       | Comment on hunk (sends to agent)  |
| `j`/`k`   | Navigate to next/previous hunk    |
| `q`/`Esc` | Exit patch mode                   |
| `Ctrl+c`  | Quit dashboard                    |

### Staging hunks

Press `y` to stage the current hunk (adds it to the git index) and advance to the next. Press `n` to skip without staging. The counter in the header shows your progress through all hunks (e.g., `[3/10]`).

After staging or skipping all hunks, the diff refreshes to show any remaining unstaged changes.

### Splitting hunks

Press `s` to split the current hunk into smaller pieces. This works when there are context lines (unchanged lines) between separate changes within a hunk. If the hunk cannot be split further, nothing happens.

### Undo

Press `u` to undo the last staged hunk. This uses `git apply --cached --reverse` to unstage it. You can undo multiple times to unstage several hunks.

### Commenting on hunks

Press `c` to enter comment mode. Type your message and press `Enter` to send it to the agent. The comment includes:
- File path and line number
- The diff hunk as context (in a code block)
- Your comment text

Press `Esc` to cancel without sending.

This is useful for giving the agent feedback about specific changes, like "This function should handle the error case" or "Can you add a test for this?"

## Sort modes

Press `s` to cycle through sort modes:

- **Priority** (default): Waiting > Done > Working > Stale
- **Project**: Group by project name, then by priority within each project
- **Recency**: Most recently updated first
- **Natural**: Original tmux order (by pane creation)

Your sort preference persists in the tmux session.

See the [Dashboard guide](/guide/dashboard) for more details.
