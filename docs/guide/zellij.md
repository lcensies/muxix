---
description: Use Zellij as an alternative multiplexer backend
---

# Zellij

::: warning Experimental — requires Zellij built from source
The Zellij backend is new and experimental. It depends on unreleased Zellij features, so you must [build Zellij from source](#building-zellij-from-source). Expect rough edges and potential issues.
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

- Zellij built from source (uses unreleased features: `--pane-id` targeting, `close-tab-by-id`, `go-to-tab-by-id`, tab ID APIs). These will ship in a future release after 0.43.
- Unix-like OS (named pipes for handshakes)
- Windows is **not supported**

### Building Zellij from source

```bash
git clone https://github.com/zellij-org/zellij.git
cd zellij
cargo build --release
# optionally install to PATH
cargo install --path .
```

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
