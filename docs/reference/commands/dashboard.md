---
description: Open a TUI dashboard showing all active AI agents
---

# dashboard

Opens a TUI dashboard showing all active AI agents across all tmux sessions.

```bash
workmux dashboard
```

## Options

- `-P, --preview-size <10-90>`: Set preview pane size as percentage (larger = more preview, less table). Default: 60.

## Examples

```bash
# Open dashboard with default layout
workmux dashboard

# Open with smaller preview pane (40% of height)
workmux dashboard --preview-size 40
```

See the [Dashboard guide](/guide/dashboard/) for keybindings and detailed documentation.
