---
description: A persistent agent status sidebar for tmux windows
---

# Sidebar

The sidebar provides an always-visible agent overview pinned to the left edge or
top edge of every tmux window. Unlike the dashboard, which is a full-screen TUI
you open on demand, the sidebar stays on screen while you work.

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/sidebar.webp" alt="muxix sidebar" style="border-radius: 4px;">
</div>

## Setup

::: warning Prerequisites
The sidebar requires [status tracking hooks](/guide/status-tracking) to be
configured and tmux as the backend.
:::

Toggle the sidebar with:

```bash
muxix sidebar            # All sessions (default)
muxix sidebar --session  # Current session only, or opt out of global mode
```

By default, the sidebar appears in all existing and newly created tmux windows
across all sessions. Use `--session` to scope it to the current session only,
leaving other sessions untouched. Running the command again disables it.

When the global sidebar is active, `muxix sidebar --session` hides it in the
current tmux session only. Run it again to show the sidebar in that session
again without affecting other sessions.

Optionally, add a tmux binding for quick access:

```bash
bind C-t run-shell "muxix sidebar"
```

## What it shows

Each agent is displayed as a tile showing:

- Status icon with spinner animation (working/waiting/done)
- Worktree name and elapsed time since last status change
- Project name and git diff stats (committed + uncommitted lines)
- Optional pull request number and check status for the current branch
- Agent task description

Agents are grouped per project: each group is sorted by activity and preceded
by a divider line with the project name (hidden when only one project is
present).

The exact layout, styling, and per-agent icons are fully customizable; see
[Customization](./customization).

## Configuration

Configure the sidebar in your global `~/.config/muxix/config.yaml` or project
`.muxix.yaml`:

```yaml
sidebar:
  # Position: "left" (default) or "top"
  position: left

  # Left sidebar width: absolute columns or percentage of terminal width
  width: 40 # absolute columns
  # width: "15%"  # percentage of terminal width

  # Layout mode for the left sidebar: "compact" or "tiles" (default)
  layout: tiles

  # How 'd' asks: "native" (default) or "popup"
  confirm_style: native

  # Prefilled into the create form that 'a' opens
  new_agent:
    placement: window # "window" (default), "session" or "pane"
    worktree: true
```

For a horizontal top bar:

```yaml
sidebar:
  position: top

  # Top bar height in rows
  height: 3

  horizontal:
    item_width: 24 # horizontal chip width in columns

  templates:
    horizontal:
      - "{status_icon} {primary} {pane_suffix} {fill} {elapsed}"
      - "{secondary} {fill} {git_stats}"
      - "{pane_title}"
```

Width defaults to 10% of terminal width, clamped between 25 and 50 columns.
When set explicitly, the clamp is removed (minimum 10 columns). Height only
applies to `position: top`; set it as a row count for the number of horizontal
lines you want to show. Horizontal item width defaults to 24 columns and is
clamped between 12 and 80. Position changes take effect the next time you toggle
the sidebar off and on.

## Layout modes

The left sidebar supports two layout modes, toggled with `v`:

- **Tiles** (default): variable-height cards with status stripe
- **Compact**: single line per agent

Your preference is persisted across tmux restarts. The top bar always uses a
horizontal chip layout, so `v` has no effect there. Horizontal templates render
as many configured lines as the current height allows.

## Mouse support

Click an agent tile or top-bar chip to jump to its pane, or scroll to navigate
the list. Requires `set -g mouse on` in your `~/.tmux.conf`.

## Keybindings

| Key               | Action                          |
| ----------------- | ------------------------------- |
| `j`/`k`           | Navigate up/down (count: `2j`)  |
| `Enter`           | Jump to agent pane              |
| `gg`/`G`          | Jump to first/last (`3G` = 3rd) |
| `Ctrl+d`/`Ctrl+u` | Half page down/up               |
| `v`     | Toggle layout mode       |
| `z`     | Toggle sleeping on agent |
| `d`     | Delete agent. Under tmux asks on the tmux prompt line (Enter or `y` confirms); elsewhere in the sidebar (Enter confirms). Any other key cancels. |
| `a`     | New agent (create form)  |
| `?`     | Show actions             |
| `q`     | Quit sidebar             |

### Sleeping agents

Press `z` to manually mark an agent as sleeping. Sleeping agents show a 💤 icon
with dimmed colors and are pushed to the bottom of the list, regardless of their
actual status. This is useful for temporarily deprioritizing agents you don't
need to monitor. Press `z` again to wake them up.

### Exited agents

By default an agent is forgotten the moment its pane is gone. Set
`persist_agents: true` to keep it instead, marked **exited** (⏏): its process
is gone but its session is not, so it can still be resumed here or picked up
from an agent manager's phone or desktop client. Exited is distinct from done,
which is a finished turn with the process still running.

```yaml
persist_agents: true   # default: false
```

A manager's session you took over locally shows the takeover's live status while
it runs, and **exited** once you quit it — even though the manager itself still
reports `idle` — until the manager reports a turn running, say one sent from
your phone. See
[`on_takeover_exit`](/guide/agent-runtimes#taking-a-session-over-natively) to follow the
manager's status instead.

### Deleting agents

Press `d` to delete the selected agent. A prompt names the agent and everywhere
it will be removed from; **Enter** deletes and **any other key** cancels, so a
stray Enter after moving the selection cannot delete the wrong one.

Delete is final. The reference is recorded as deleted, and every path that
lists, adopts, opens or restores agents honours that record — so a manager whose
delete is asynchronous, or whose listing muxix has cached, cannot put the row
back. The record is dropped once the thing it names is gone, so it does not
accumulate.

- A local agent's record is removed, and its pane killed whenever a live
  process is still there. The *recorded* status is not consulted: a record reads
  `Exited` as soon as the pane's foreground command stops matching, which also
  happens while the agent is alive. What decides is whether the pane is still
  the one this agent recorded (pane id **and** pid), so a recycled pane id is
  never killed and a pane that is genuinely gone is not an error.
- Deleting also clears what the agent would still be recognised by: its
  multiplexer status markers, its pane-keyed signal files, and the session id
  journalled for its worktree — so a later `muxix open --continue` starts fresh
  instead of resuming the conversation you deleted. Stored task prompts
  (`.muxix/PROMPT-*.md`) are left alone; they are your text, not an identity.
- A manager's agent is deleted in the manager too, through its `delete_args`
  (paseo: `agent delete`, a hard delete — gone from every device). If the
  manager refuses, the failure is reported but the agent still disappears from
  muxix: it was deleted here, and leaving the row visible is what reopened it.
  A manager with no `delete_args` keeps its copy.
- Activating a row the manager no longer lists reports that and opens nothing.
  It used to create a session, a window and a view record for an agent that did
  not exist.

### The confirmation popup

By default the delete prompt is the multiplexer's own: under tmux it appears on
the prompt line, outside the sidebar pane, and stays until answered. Set
`confirm_style: popup` to get a centered dialog drawn inside the sidebar pane
instead; the native prompt is then never invoked.

```yaml
sidebar:
  confirm_style: popup
```

The agent is captured when the dialog opens, so a refresh that reorders the list
cannot retarget the delete. `Enter` or `y` deletes, every other key cancels. The
`top` position is one to five rows tall — too short for a dialog — so it keeps
the one-line prompt band whatever the style is.

### Creating an agent

Press `a` to create a new agent without leaving the sidebar. A form opens in the
pane, prefilled from the selected row and from `sidebar.new_agent`:

| Field | Prefilled with |
| ----- | -------------- |
| Branch | empty — type the branch to create |
| Repository | the selected agent's repository, re-derived from its pane's working directory |
| Agent | empty, so the repository's own [`agent_rules`](/guide/agents) choose the profile |
| Placement | `sidebar.new_agent.placement` |
| Worktree | `sidebar.new_agent.worktree` |

`Tab` and `Shift+Tab` move between fields, `←`/`→` cycle a choice field, `Enter`
submits and `Esc` cancels. The form still opens with nothing selected, with the
repository field empty — it is required, so fill it in.

Placement `pane` splits the selected agent's pane instead of opening a window,
so it needs a selected row. Worktree `no` runs the agent on the repository
root's existing checkout: no branch and no worktree are created, and no branch
name is required. File operations and `post_create` hooks are skipped in that
case, since they exist to provision a fresh checkout, and what was skipped is
reported. It is the same path as
[`muxix add --no-worktree`](/reference/commands/add).

Creation runs on a background thread, so the sidebar stays responsive, and the
outcome — the created handle or the error — is reported when it finishes.

### The action list

Press `?` for a filterable list of the sidebar's own actions, built from the same
binding table as the keys above. Type to filter, `Enter` runs the selected
action. An action that needs a selected agent reports that and does nothing when
nothing is selected.

## Agent navigation hotkeys

You can switch between agents from any tmux pane using subcommands. These work
in the same order shown in the sidebar:

| Command                    | Action                               |
| -------------------------- | ------------------------------------ |
| `muxix sidebar next`     | Switch to the next agent (wraps)     |
| `muxix sidebar prev`     | Switch to the previous agent (wraps) |
| `muxix sidebar jump <N>` | Jump to the Nth agent (1-indexed)    |

### Example tmux keybindings

```bash
# Alt+j / Alt+k to cycle agents (no prefix needed)
bind -n M-j run-shell "muxix sidebar next"
bind -n M-k run-shell "muxix sidebar prev"

# Alt+1..9 to jump directly
bind -n M-1 run-shell "muxix sidebar jump 1"
bind -n M-2 run-shell "muxix sidebar jump 2"
bind -n M-3 run-shell "muxix sidebar jump 3"
# ...
```

## How it works

The sidebar is a bit of a hack on top of tmux's pane system, but it works quite
well. It uses a daemon + client architecture with event-driven rendering:

1. **Toggle on** (`muxix sidebar`): creates a tmux pane on the left or top
   edge of every window, starts a background daemon, and installs tmux hooks.

2. **Daemon**: a single headless process that polls tmux state every 2 seconds
   (or immediately when signaled via SIGUSR1). It reads agent state from the
   filesystem, queries tmux for pane geometry and active windows, then pushes
   snapshots to all connected sidebar clients over a Unix socket.

3. **Clients**: every tmux window gets its own sidebar pane running a separate
   `muxix _sidebar-run` process. Each process connects to the shared daemon
   socket, receives snapshots via a background reader thread, and renders
   independently. The main thread blocks on a channel, only waking when new
   data arrives or a spinner tick is needed. Rendering is skipped entirely for
   inactive windows. This event-driven design keeps CPU usage near zero when
   idle.

4. **Hooks**: tmux hooks handle lifecycle events:
   - `after-new-window` / `after-new-session`: automatically adds a sidebar pane
     to newly created windows
   - `window-resized`: reflows the layout tree in every sidebar window, keeping
     all sidebars at the correct width or height regardless of which window was
     resized
   - `after-select-window` / `client-session-changed` / `after-kill-pane`:
     signals the daemon for an immediate refresh

5. **Layout reflow**: when the sidebar is added or the terminal is resized, a
   layout tree parser reads the tmux `#{window_layout}` string, scales the
   content subtree proportionally, and applies the result atomically via
   `select-layout`. This preserves existing pane proportions (e.g. a 70/30 split
   stays 70/30).

6. **Toggle off** (`muxix sidebar` again, or `q` in any sidebar): kills all
   sidebar panes, reflows content panes to fill the freed space, stops the
   daemon, and removes hooks.

7. **A window emptying out**: when the sidebar is the only pane left in its
   window — the agent, session view or shell beside it ended — that one
   sidebar exits so the window can close. Sidebars in other windows are not
   affected.

### Resource usage

Because tmux has no concept of a pane that persists across all windows, each
window runs its own `_sidebar-run` process. Each one uses roughly 15 MB of
resident memory, and the shared daemon (`_daemon`) uses about 16 MB. With
five agents running, total memory footprint is around 90 MB. CPU usage is near
zero when idle thanks to the event-driven architecture.
