---
description: Query agent status and completion state for worktrees
---

# status

Shows current agent status for one or more worktrees: turn-boundary status (`working` / `waiting` / `done`), elapsed time, and, once an agent has signalled it, its completion state.

```bash
workmux status [name...] [flags]
```

## Arguments

- `[name...]`: worktree names (supports cross-project `project:handle` syntax). Omit to show all active agents in the current repo.

## Options

| Flag     | Description                                                        |
| -------- | ------------------------------------------------------------------- |
| `--json` | Output as JSON instead of a table.                                   |
| `--git`  | Include git info (staged/unstaged changes, unmerged commits).        |

## Completion in the STATUS column

Once an agent has run [`workmux signal done`](./signal) or `workmux signal error`, the STATUS column appends the completion: `done · completed` or `working · failed`. The turn status and the completion are independent — an agent can show `working · completed` if it kept running (e.g. tool-call cleanup) after signalling done.

## `--json` output

Each entry is a `StatusEntry` with `worktree`, `branch`, `status`, `elapsed_secs`, `title`, `pane_id`, optional `git`, and — once signalled — a `completion` object:

```json
{
  "worktree": "user-auth",
  "branch": "user-auth",
  "status": "done",
  "elapsed_secs": 42,
  "title": "Implement login flow",
  "pane_id": "%3",
  "completion": {
    "kind": "completed",
    "feedback": "tests pass",
    "ts": 1767000000
  }
}
```

`completion` is omitted entirely when the agent has not signalled `done`/`error` yet (or after a clear on relaunch/send — see [`workmux signal`](./signal)).

## Examples

```bash
# Show all active agents in the current repo
workmux status

# Show status for specific worktrees
workmux status user-auth api-refactor

# Include git info
workmux status user-auth --git

# Machine-readable output, e.g. for a coordinator script
workmux status user-auth --json
```

## Related

- [`workmux signal`](./signal) — how the completion record shown here gets written.
- [`workmux wait`](./wait) — block until status/completion reaches a target instead of polling.
