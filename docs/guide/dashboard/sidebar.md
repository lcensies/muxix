---
description: A persistent agent status sidebar for tmux windows
---

# Sidebar

The sidebar provides an always-visible agent overview pinned to the left edge of
every tmux window. Unlike the dashboard (which is a full-screen TUI you open on
demand), the sidebar stays on screen while you work.

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/sidebar.webp" alt="workmux sidebar" style="border-radius: 4px;">
</div>

## Setup

::: warning Prerequisites
The sidebar requires [status tracking hooks](/guide/status-tracking) to be
configured and tmux as the backend.
:::

Toggle the sidebar with:

```bash
workmux sidebar            # All sessions (default)
workmux sidebar --session  # Current session only
```

By default, the sidebar appears in all existing and newly created tmux windows
across all sessions. Use `--session` to scope it to the current session only,
leaving other sessions untouched. Running the command again disables it.

Optionally, add a tmux binding for quick access:

```bash
bind C-t run-shell "workmux sidebar"
```

## What it shows

Each agent is displayed as a tile showing:

- Status icon with spinner animation (working/waiting/done)
- Worktree name and elapsed time since last status change
- Project name and git diff stats (committed + uncommitted lines)
- Agent task description

## Configuration

Configure the sidebar in your global `~/.config/workmux/config.yaml` or project
`.workmux.yaml`:

```yaml
sidebar:
  # Width: absolute columns or percentage of terminal width
  width: 40 # absolute columns
  # width: "15%"  # percentage of terminal width

  # Layout mode: "compact" or "tiles" (default)
  layout: tiles
```

Width defaults to 10% of terminal width, clamped between 25 and 50 columns.
When set explicitly, the clamp is removed (minimum 10 columns).

## Templates

The sidebar renders each agent row from a small token-based template DSL. You
can override the built-in templates per layout mode:

```yaml
sidebar:
  templates:
    # Compact mode: a single line per agent.
    compact: "{status_icon} {primary} {pane_suffix} {fill} {elapsed}"

    # Tile mode: one string per visual line in the tile body.
    tiles:
      - "{primary} {pane_suffix} {fill} {elapsed}"
      - "{secondary} {fill} {git_stats}"
      - "{pane_title}"
```

The values shown above are also the built-in defaults, so leaving these keys
unset gives you the standard rendering.

Templates can be set in either the global config or a project's `.workmux.yaml`.
Project values override global values. Changes are picked up live by running
sidebars without a restart.

### Tokens

| Token            | Description                                                                                                                |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------- |
| `{primary}`      | Primary identity label (worktree / window / session / project chain).                                                      |
| `{secondary}`    | Secondary label from the same chain, with worktree appended if not already primary.                                        |
| `{worktree}`     | Worktree directory name.                                                                                                   |
| `{project}`      | Project name (parent of the worktree).                                                                                     |
| `{session}`      | Tmux session name (blank for workmux-prefixed sessions).                                                                   |
| `{window}`       | Tmux window name (blank for generic shell names like `zsh`, `bash`).                                                       |
| `{pane_title}`   | Sanitized agent task title from the pane title.                                                                            |
| `{pane_suffix}`  | Disambiguator like `(1)`, `(2)` when multiple agents share a window. Empty otherwise.                                      |
| `{status_icon}`  | Status indicator (working spinner, waiting, done, sleeping, etc.).                                                         |
| `{agent_icon}`   | Per-agent icon based on the running agent's profile (see below).                                                           |
| `{agent_label}`  | Capitalized agent name (e.g. `Claude`, `Codex`).                                                                           |
| `{elapsed}`      | Elapsed time since the agent's last status change.                                                                         |
| `{git_stats}`    | Composite git diff stats: committed (`+1278 -400`), pen icon, uncommitted (`+21`). Self-degrades to fit the space it gets. |
| `{git_branch}`   | Current branch name. Empty when detached HEAD or git status unavailable.                                                   |
| `{git_ahead}`    | Commits ahead of upstream as `↑N` when N greater than 0. Empty when 0 or no upstream.                                      |
| `{git_behind}`   | Commits behind upstream as `↓N` when N greater than 0. Empty when 0 or no upstream.                                        |
| `{git_dirty}`    | Diff glyph when the working tree is dirty. Empty when clean.                                                               |
| `{git_conflict}` | Conflict glyph when the worktree has merge conflicts. Empty otherwise.                                                     |
| `{status_label}` | Display name for the agent status: `Working`, `Waiting`, `Done`, or empty when no status.                                  |
| `{idx}`          | 1-based sidebar position (`1`, `2`, ...).                                                                                  |
| `{jump_key}`     | The `M-1`..`M-9` chord label for the first nine rows. Empty for row 10 and beyond.                                         |
| `{fill}`         | Layout marker that splits a line into a left and right segment. At most one per line.                                      |

`{git_ahead}` and `{git_behind}` already include the arrow prefix, so do not
wrap them with another `↑` / `↓` literal in your template -- a stray glyph
would remain when the count is zero. The same applies to `{git_dirty}` and
`{git_conflict}`, which are self-contained glyph indicators.

Unknown tokens or unbalanced braces cause the template to be rejected and the
previous valid template (or the built-in default) is kept.

### Layout

`{fill}` is the only layout marker. Tokens before it form the left segment and
tokens after it form the right segment. The leftmost flex token in the left
segment absorbs ellipsis-truncation when there isn't enough room. Flex tokens
are: `{primary}`, `{secondary}`, `{worktree}`, `{project}`, `{session}`,
`{window}`, `{pane_title}`. Other tokens always render at their natural width.

When a line has more slack than the flex token needs, the leftover is emitted as
spaces between the left and right segments, so right-segment tokens like
`{elapsed}` line up against the right edge.

When a token resolves to an empty string, one adjacent literal whitespace is
dropped automatically. This means `{primary} {pane_suffix} {fill} {elapsed}`
renders cleanly whether or not `{pane_suffix}` is empty.

In tile mode, the stripe, status icon column, and a 1-column right margin are
drawn as chrome by the renderer. Templates only control the body area, so line
alignment between tiles is automatic. A tile line that contains no fields with
content (only literals or empty fields) is dropped, so optional lines like the
default `{pane_title}` row collapse when there's nothing to show.

### Escaping

Use <code v-pre>{{</code> for a literal `{` and <code v-pre>}}</code> for a
literal `}`.

### Agent identity

Adding `{agent_icon}` or `{agent_label}` to a template surfaces which agent is
running in each pane. Identity is detected from the stored agent command via
the same profile system used elsewhere in workmux.

Default icons: `claude` → `CC`, `codex` → `CX`, `opencode` → `OC`, `gemini` →
`G`, `pi` → `π`, `kiro-cli` → `K`, `vibe` → `V`, `copilot` → `CP`. Unknown
agents render an empty icon.

Default colors are brand accents: Claude orange, Codex teal, Gemini blue,
Copilot purple, Vibe orange, Pi sage, OpenCode blue. Stale rows still dim
and selected rows still take the highlight background; the icon color sits
on top of those.

Override icons or colors per agent under `sidebar.agent_icons`. Each value
is either a bare string (icon only, default color stays) or an object with
`icon` and `color`. Color values use the same format as `theme.custom`:
hex (`'#ff8c00'`), named ANSI (`red`, `yellow`, `lightgreen`), or indexed
(`'214'`).

```yaml
sidebar:
  agent_icons:
    # Bare string: icon only, default brand color stays
    vibe: V

    # Override color only
    gemini:
      color: cyan

    # Override both
    claude:
      icon: CC
      color: '#ff8c00'

    # Disable the default color (use palette text color)
    codex:
      color: ''
```

### Status icons

The `{status_icon}` token renders the working spinner, waiting indicator,
done check, and sleeping indicator. Defaults depend on whether you have
[nerdfont](/guide/nerdfont) enabled:

| State    | Default (no nerdfont) | Default (nerdfont) |
| -------- | --------------------- | ------------------ |
| Working  | braille spinner       | braille spinner    |
| Waiting  | 💬                    | nf-fa-comment      |
| Done     | ✅                    | nf-md-check_circle |
| Sleeping | 💤                    | nf-md-sleep        |

Override per state under top-level `status_icons`. Any value set here
wins over both the emoji and nerdfont defaults, so you can mix and match.
Setting `working` also replaces the braille spinner with a static icon:

```yaml
status_icons:
  working: "🤖"
  waiting: "💬"
  done: "✅"
```

### Examples

Show only the worktree and elapsed time per agent in compact mode:

```yaml
sidebar:
  layout: compact
  templates:
    compact: "{status_icon} {worktree} {fill} {elapsed}"
```

Add the agent icon next to the primary label in tile mode:

```yaml
sidebar:
  templates:
    tiles:
      - "{agent_icon} {primary} {pane_suffix} {fill} {elapsed}"
      - "{secondary} {fill} {git_stats}"
      - "{pane_title}"
```

Drop the third line entirely (no pane title) and show git stats inline on line
one:

```yaml
sidebar:
  templates:
    tiles:
      - "{primary} {fill} {git_stats}"
      - "{secondary} {fill} {elapsed}"
```

## Layout modes

The sidebar supports two layout modes, toggled with `v`:

- **Tiles** (default): variable-height cards with status stripe
- **Compact**: single line per agent

Your preference is persisted across tmux restarts.

## Mouse support

Click an agent tile to jump to its pane, or scroll to navigate the list. Requires
`set -g mouse on` in your `~/.tmux.conf`.

## Keybindings

| Key     | Action                   |
| ------- | ------------------------ |
| `j`/`k` | Navigate up/down         |
| `Enter` | Jump to agent pane       |
| `g`/`G` | Jump to first/last       |
| `v`     | Toggle layout mode       |
| `z`     | Toggle sleeping on agent |
| `q`     | Quit sidebar             |

### Sleeping agents

Press `z` to manually mark an agent as sleeping. Sleeping agents show a 💤 icon
with dimmed colors and are pushed to the bottom of the list, regardless of their
actual status. This is useful for temporarily deprioritizing agents you don't
need to monitor. Press `z` again to wake them up.

## Agent navigation hotkeys

You can switch between agents from any tmux pane using subcommands. These work
in the same order shown in the sidebar:

| Command                    | Action                               |
| -------------------------- | ------------------------------------ |
| `workmux sidebar next`     | Switch to the next agent (wraps)     |
| `workmux sidebar prev`     | Switch to the previous agent (wraps) |
| `workmux sidebar jump <N>` | Jump to the Nth agent (1-indexed)    |

### Example tmux keybindings

```bash
# Alt+j / Alt+k to cycle agents (no prefix needed)
bind -n M-j run-shell "workmux sidebar next"
bind -n M-k run-shell "workmux sidebar prev"

# Alt+1..9 to jump directly
bind -n M-1 run-shell "workmux sidebar jump 1"
bind -n M-2 run-shell "workmux sidebar jump 2"
bind -n M-3 run-shell "workmux sidebar jump 3"
# ...
```

## How it works

The sidebar is a bit of a hack on top of tmux's pane system, but it works quite
well. It uses a daemon + client architecture with event-driven rendering:

1. **Toggle on** (`workmux sidebar`): creates a narrow tmux pane on the left
   side of every window using a full-height split, starts a background daemon,
   and installs tmux hooks.

2. **Daemon**: a single headless process that polls tmux state every 2 seconds
   (or immediately when signaled via SIGUSR1). It reads agent state from the
   filesystem, queries tmux for pane geometry and active windows, then pushes
   snapshots to all connected sidebar clients over a Unix socket.

3. **Clients**: every tmux window gets its own sidebar pane running a separate
   `workmux _sidebar-run` process. Each process connects to the shared daemon
   socket, receives snapshots via a background reader thread, and renders
   independently. The main thread blocks on a channel, only waking when new
   data arrives or a spinner tick is needed. Rendering is skipped entirely for
   inactive windows. This event-driven design keeps CPU usage near zero when
   idle.

4. **Hooks**: tmux hooks handle lifecycle events:
   - `after-new-window` / `after-new-session`: automatically adds a sidebar pane
     to newly created windows
   - `window-resized`: reflows the layout tree to keep the sidebar at the
     correct width and content panes proportionally balanced
   - `after-select-window` / `client-session-changed` / `after-kill-pane`:
     signals the daemon for an immediate refresh

5. **Layout reflow**: when the sidebar is added or the terminal is resized, a
   layout tree parser reads the tmux `#{window_layout}` string, scales the
   content subtree proportionally, and applies the result atomically via
   `select-layout`. This preserves existing pane proportions (e.g. a 70/30 split
   stays 70/30).

6. **Toggle off**: kills all sidebar panes, reflows content panes to fill the
   freed space, stops the daemon, and removes hooks.

## Resource usage

Because tmux has no concept of a pane that persists across all windows, each
window runs its own `_sidebar-run` process. Each one uses roughly 15 MB of
resident memory, and the shared daemon (`_sidebar-daemon`) uses about 16 MB. With
five agents running, total memory footprint is around 90 MB. CPU usage is near
zero when idle thanks to the event-driven architecture.
