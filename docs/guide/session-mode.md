---
description: Create worktrees in their own tmux sessions instead of windows
---

# Session mode

By default, workmux creates tmux **windows** within your current session. With session mode, each worktree gets its own **tmux session** instead.

This is useful when you want each worktree to have multiple windows, or when you prefer the isolation of separate sessions (each with its own window list, history, and layout).

## Enabling session mode

Per-project via config:

```yaml
# .workmux.yaml
mode: session
```

Globally via config:

```yaml
# ~/.config/workmux/config.yaml
mode: session
```

Or per-worktree via flag:

```bash
workmux add feature-branch --session
```

The `--session` flag overrides the config for that specific worktree. This lets you use window mode by default but create individual worktrees as sessions when needed.

## How it works

- **Persistence**: The mode is stored per-worktree in git config. Once a worktree is created with session mode, `open`, `close`, `remove`, and `merge` automatically use the correct mode.
- **Navigation**: `workmux add` switches your client to the new session. `merge` and `remove` switch you back to the previous session.

## Multiple windows per session

Use the `windows` config to create multiple windows in each session. Each window can have its own pane layout. This is mutually exclusive with the top-level `panes` config.

```yaml
mode: session
windows:
  - name: editor
    panes:
      - command: <agent>
        focus: true
      - split: horizontal
        size: 20
  - name: tests
    panes:
      - command: just test --watch
  - panes:
      - command: tail -f app.log
```

Each window supports:

| Option  | Description                                            | Default      |
| ------- | ------------------------------------------------------ | ------------ |
| `name`  | Window name (if omitted, tmux auto-names from command) | Auto         |
| `panes` | Pane layout (same syntax as top-level `panes`)         | Single shell |

Named windows keep their name permanently. Unnamed windows use tmux's automatic naming based on the running command.

`focus: true` works across windows: the last pane with focus set determines which window is active when the session opens.

## Limitations

- **tmux only**: Session mode is only supported for the tmux backend. WezTerm, kitty, and Zellij do not support sessions.
- **No duplicates**: Unlike window mode which supports opening multiple windows for the same worktree (with `-2`, `-3` suffixes), session mode creates one session per worktree.
