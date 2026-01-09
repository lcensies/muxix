# Dashboard

When running multiple AI agents in parallel, it's helpful to have a centralized view of what each agent is doing. The dashboard provides a TUI for monitoring all active agents across all tmux sessions.

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/dashboard.webp" alt="workmux dashboard" style="border-radius: 4px;">
</div>

## Setup

Add this binding to your `~/.tmux.conf`:

```bash
bind C-s display-popup -h 30 -w 100 -E "workmux dashboard"
```

Then press `prefix + Ctrl-s` to open the dashboard as a tmux popup. Feel free to adjust the keybinding and popup dimensions (`-h` and `-w`) as needed.

::: warning Prerequisites
This feature requires [status tracking hooks](/guide/status-tracking) to be configured. Without them, no agents will appear in the dashboard.
:::

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

## Diff view

Press `d` to view the diff for the selected agent. The diff view has two modes:

- **WIP** - Shows uncommitted changes (`git diff HEAD`)
- **review** - Shows all changes on the branch vs main (`git diff main...HEAD`)

Press `Tab` while in diff view to toggle between modes. The footer displays which mode is active along with diff statistics showing lines added (+) and removed (-).

If there are no changes to show, a message is displayed instead:

- WIP mode: "No uncommitted changes"
- Review mode: "No commits on this branch yet"

### Diff view keybindings

| Key       | Action                      |
| --------- | --------------------------- |
| `Tab`     | Toggle WIP / review         |
| `a`       | Enter patch mode (WIP only) |
| `j`/`k`   | Scroll down/up              |
| `Ctrl+d`  | Page down                   |
| `Ctrl+u`  | Page up                     |
| `c`       | Send commit action to agent |
| `m`       | Send merge action to agent  |
| `q`/`Esc` | Close diff view             |
| `Ctrl+c`  | Quit dashboard              |

## Patch mode

Patch mode (`a` from WIP diff) allows staging individual hunks like `git add -p`. This is useful for selectively staging parts of an agent's work.

When [delta](https://github.com/dandavison/delta) is installed, hunks are rendered with syntax highlighting for better readability.

### Patch mode keybindings

| Key       | Action                           |
| --------- | -------------------------------- |
| `y`       | Stage current hunk               |
| `n`       | Skip current hunk                |
| `u`       | Undo last staged hunk            |
| `s`       | Split hunk (if splittable)       |
| `o`       | Comment on hunk (sends to agent) |
| `j`/`k`   | Navigate to next/previous hunk   |
| `q`/`Esc` | Exit patch mode                  |
| `Ctrl+c`  | Quit dashboard                   |

### Staging hunks

Press `y` to stage the current hunk (adds it to the git index) and advance to the next. Press `n` to skip without staging. The counter in the header shows your progress through all hunks (e.g., `[3/10]`).

After staging or skipping all hunks, the diff refreshes to show any remaining unstaged changes.

### Splitting hunks

Press `s` to split the current hunk into smaller pieces. This works when there are context lines (unchanged lines) between separate changes within a hunk. If the hunk cannot be split further, nothing happens.

### Undo

Press `u` to undo the last staged hunk. This uses `git apply --cached --reverse` to unstage it. You can undo multiple times to unstage several hunks.

### Commenting on hunks

Press `o` to enter comment mode. Type your message and press `Enter` to send it to the agent. The comment includes:

- File path and line number
- The diff hunk as context (in a code block)
- Your comment text

Press `Esc` to cancel without sending.

This is useful for giving the agent feedback about specific changes, like "This function should handle the error case" or "Can you add a test for this?"

## Configuration

The commit (`c`) and merge (`m`) actions can be customized in your `.workmux.yaml`:

```yaml
dashboard:
  commit: "Commit staged changes with a descriptive message"
  merge: "!workmux merge"
```

Both values are text sent to the agent's pane. Use the `!` prefix to run shell commands (supported by Claude, Gemini, and other agents).

### Defaults

| Action   | Default value                                      | Description             |
| -------- | -------------------------------------------------- | ----------------------- |
| `commit` | `Commit staged changes with a descriptive message` | Natural language prompt |
| `merge`  | `!workmux merge`                                   | Shell command via agent |

### Examples

```yaml
# Use Claude slash commands (requires ~/.claude/commands/ setup)
dashboard:
  commit: "/commit"
  merge: "/merge"

# Custom shell commands
dashboard:
  merge: "!workmux merge --rebase --notification"

# Natural language prompts
dashboard:
  commit: "Create a commit with a conventional commit message"
  merge: "Rebase onto main and run workmux merge"
```

### Using slash commands

For complex workflows, [slash commands](/guide/slash-commands) are more powerful than simple prompts or shell commands. A slash command can encode detailed, multi-step instructions that the agent follows intelligently.

```yaml
dashboard:
  commit: "/commit"
  merge: "/merge"
```

See the [slash commands guide](/guide/slash-commands) for a complete `/merge` example you can copy.
