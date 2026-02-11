---
description: Use kitty as an alternative multiplexer backend
---

# Kitty backend

::: warning Experimental
The kitty backend is new and experimental. Expect rough edges and potential issues.
:::

workmux supports [kitty](https://sw.kovidgoyal.net/kitty/) as an alternative to tmux. This is useful if you prefer kitty's features or already use kitty as your terminal.

workmux automatically uses kitty when it detects the `$KITTY_WINDOW_ID` environment variable.

## Differences from tmux

| Feature              | tmux                 | kitty                  |
| -------------------- | -------------------- | ---------------------- |
| Agent status in tabs | Yes (window names)   | Yes (custom tab title) |
| Tab ordering         | Insert after current | Appends to end         |
| Scope                | tmux session         | OS window              |

- **Tab ordering**: New tabs appear at the end of the tab bar (no "insert after" support like tmux)
- **OS window isolation**: workmux operates within the current OS window. Tabs in other OS windows are not affected.
- **Terminology note**: What workmux calls a "pane" is called a "window" in kitty, and what workmux calls a "window" (tab) is called a "tab" in kitty

## Requirements

- kitty with remote control enabled (`kitten @` must work)
- Unix-like OS (named pipes for handshakes)
- Windows is **not supported**
- **Required kitty configuration** (see below)

## Required kitty configuration

workmux relies on kitty's remote control API. Add these settings to your `kitty.conf`:

```bash
# REQUIRED: Enable remote control
allow_remote_control yes

# REQUIRED: Set up socket for remote control
# The socket path can be customized, but using kitty_pid ensures uniqueness
listen_on unix:/tmp/kitty-{kitty_pid}

# RECOMMENDED: Enable splits layout for pane splitting
enabled_layouts splits,stack
```

## Verify remote control works

After configuring kitty, verify that remote control is working:

```bash
kitten @ ls
```

This should output JSON describing your kitty windows and tabs. If you get an error about remote control being disabled, check your `kitty.conf` configuration.

## Agent status display

workmux stores agent status in kitty [user variables](https://sw.kovidgoyal.net/kitty/remote-control/#kitten-set-user-vars) (`workmux_status`), which can be displayed in tab titles using kitty's `{custom}` template placeholder.

### Setup

1. Create `~/.config/kitty/tab_bar.py`:

```python
from kitty.fast_data_types import get_boss

def draw_title(data):
    tab = get_boss().tab_for_id(data['tab'].tab_id)
    if tab:
        for window in tab:
            status = window.user_vars.get('workmux_status', '')
            if status:
                return ' ' + status
    return ''
```

2. Add to your `kitty.conf`:

```bash
tab_title_template "{title}{custom}"
```

The `{custom}` placeholder calls the `draw_title` function above, which checks each window in the tab for a `workmux_status` user variable and appends it to the title.

## Known limitations

- Windows is not supported (requires Unix-specific features)
- Agent status icons require a small config change (see above)
- Cross-OS-window operations are not supported
- Some edge cases may not be as thoroughly tested as the tmux backend
- Tab insertion ordering is not supported (new tabs always appear at the end)
