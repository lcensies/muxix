---
description: Use Zellij as an alternative multiplexer backend
---

# Zellij

::: warning Experimental
The Zellij backend is new and experimental. Expect rough edges and potential issues.
:::

[Zellij](https://zellij.dev/) can be used as an alternative to tmux. Detected automatically via `$ZELLIJ`.

## Differences from tmux

| Feature              | tmux                 | Zellij             |
| -------------------- | -------------------- | ------------------ |
| Agent status in tabs | Yes (window names)   | No                 |
| Tab ordering         | Insert after current | Appends to end     |
| Scope                | tmux session         | Zellij session     |
| Session mode         | Yes                  | No (window only)   |
| Pane size control    | Percentage-based     | 50/50 splits only  |
| Dashboard preview    | Yes                  | No                 |

- **Tab ordering**: New tabs appear at the end of the tab bar (no "insert after" support like tmux)
- **Session isolation**: workmux operates within the current Zellij session. Tabs in other sessions are not affected.
- **Window mode only**: Session mode (`--session`) is not supported. Use window mode instead.
- **Pane splits**: All splits are 50/50 — percentage-based sizing is not available via the Zellij CLI.
- **No dashboard preview**: Zellij's `dump-screen` only captures the focused pane, so preview in the dashboard is disabled.

## Requirements

- Zellij **0.44.0** or later (uses tab ID APIs and `--pane-id` targeting)
- Unix-like OS (named pipes for handshakes)
- Windows is **not supported**

## Configuration

No special Zellij configuration is required. workmux uses Zellij's built-in CLI actions (`zellij action`) which work out of the box.

If you want to override the auto-detected backend, set the `WORKMUX_BACKEND` environment variable:

```bash
export WORKMUX_BACKEND=zellij
```

## Known limitations

- Windows is not supported (requires Unix-specific features)
- Session mode is not supported — only window mode works
- Agent status icons do not appear in tab titles
- Dashboard preview pane is disabled (captures focused pane only)
- Pane splits are always 50/50 (no percentage-based sizing)
- Tab insertion ordering is not supported (new tabs always appear at the end)
- Some edge cases may not be as thoroughly tested as the tmux backend
