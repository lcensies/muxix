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

| Feature              | tmux                 | kitty             |
| -------------------- | -------------------- | ----------------- |
| Agent status in tabs | Yes (window names)   | Dashboard only    |
| Tab ordering         | Insert after current | Appends to end    |
| Scope                | tmux session         | OS window         |

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

```conf
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

Unlike tmux, kitty does not have built-in support for displaying status icons in tab titles. workmux stores agent status in kitty user variables (`workmux_status`), which can be read by custom tab bar scripts.

To display status icons in your tab bar, you can create a custom `tab_bar.py`:

```python
# ~/.config/kitty/tab_bar.py
from kitty.tab_bar import DrawData, ExtraData, TabBarData, as_rgb, draw_title

def draw_tab(
    draw_data: DrawData, screen: DrawData.screen_class, tab: TabBarData,
    before: int, max_title_length: int, index: int, is_last: bool,
    extra_data: ExtraData
) -> int:
    # Check for workmux status in any window
    status = ''
    for window in tab.windows:
        if hasattr(window, 'user_vars') and 'workmux_status' in window.user_vars:
            status = window.user_vars['workmux_status'] + ' '
            break

    # Draw status + title
    title = status + tab.title
    return draw_title(draw_data, screen, tab, title, max_title_length, index, is_last, extra_data)
```

Then enable it in `kitty.conf`:

```conf
tab_bar_style custom
tab_bar_custom draw_tab
```

## Known limitations

- Windows is not supported (requires Unix-specific features)
- Agent status icons require custom tab bar configuration (see above)
- Cross-OS-window operations are not supported
- Some edge cases may not be as thoroughly tested as the tmux backend
- Tab insertion ordering is not supported (new tabs always appear at the end)
