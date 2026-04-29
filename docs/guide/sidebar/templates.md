---
description: Customize the sidebar layout with a token-based template DSL
---

# Templates

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

## Tokens

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
| `{agent_icon}`   | Per-agent icon based on the running agent's profile (see [Agent identity](#agent-identity)).                               |
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
wrap them with another `↑` / `↓` literal in your template, otherwise a stray
glyph would remain when the count is zero. The same applies to `{git_dirty}`
and `{git_conflict}`, which are self-contained glyph indicators.

Unknown tokens or unbalanced braces cause the template to be rejected and the
previous valid template (or the built-in default) is kept.

## Layout

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

## Escaping

Use <code v-pre>{{</code> for a literal `{` and <code v-pre>}}</code> for a
literal `}`.

## Agent identity

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
      color: "#ff8c00"

    # Disable the default color (use palette text color)
    codex:
      color: ""
```

A project-level `agent_icons` map merges into the global one per agent
key. Setting `claude:` in a project replaces only the global `claude`
entry; other agents defined in the global config are preserved. To
clear an inherited override entirely for one agent, set its value to
`null` (e.g. `claude: ~`).

## Status icons

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

## Examples

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
