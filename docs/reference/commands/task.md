---
description: Read and mutate the project's task graph from the shell
---

# `workmux task`

CRUD over the project's task graph — a JSON array of tasks in
`tasks/index.json` by default. Humans and agents read and mutate it from a shell
instead of hand-editing JSON; every write goes through the same atomic, locked
path, so concurrent agents cannot clobber each other.

workmux owns the **store**, not the schedule. There is no loop that claims tasks
and spawns agents: a harness reads the ready frontier, decides what to run,
creates the worktree with [`workmux add`](./add), and writes the outcome back.
The [dashboard](./dashboard) `Tasks` tab is a view over the same file.

Relative `--graph` paths resolve against the **main worktree root**, so an agent
working inside a feature worktree reads and writes the project's single graph
rather than a per-worktree copy.

## Subcommands

| Command                   | What                                                                    |
| ------------------------- | ----------------------------------------------------------------------- |
| `task list`               | List tasks; filter by status/label, or show only the ready frontier     |
| `task get <query>`        | One task by exact id, else a fuzzy search over id + title               |
| `task create`             | Add a task (agents use this to record discovered work)                  |
| `task update <id>`        | Change fields; only the flags you pass are touched                      |
| `task delete <id>`        | Remove a task                                                           |
| `task claim <id>`         | Bind a task to the worktree and branch you are about to create          |
| `task resolve <id>`       | Settle a task whose claimed slot is `unknown` (`--retry` / `--abandon`) |

## Task schema

```json
[
  {
    "id": "backend",
    "title": "REST API",
    "description": "CRUD endpoints for /api/items",
    "status": "todo",
    "depends_on": [],
    "labels": ["backend"],
    "priority": 1,
    "worktree": "feature-api"
  }
]
```

`status` is `todo | in_progress | merging | done | failed | blocked`. A task is
on the **ready frontier** when it is `todo` and every id in `depends_on` is
`done`.

## Reading

```bash
workmux task list                              # table of everything
workmux task list --status todo --label backend
workmux task list --ready --json               # the frontier, for a harness
workmux task get backend                       # exact id
workmux task get "rest api"                    # fuzzy over id + title
```

`--ready` and `--frontier` are the same filter under two names.

## Writing

```bash
# record work discovered mid-task
workmux task create --title "Rate-limit /api/items" --label backend --depends-on backend

# move it along (only the flags you pass change)
workmux task update backend --status in_progress
workmux task update backend --add-labels needs-review --hint "watch the N+1 in list()"
workmux task update backend --status done

workmux task delete backend
```

## Claiming a slot

A harness that is about to create a worktree for a task records the binding
first, so a crash between "decided" and "created" is recoverable:

```bash
workmux task claim backend \
  --branch feat/api --base main --worktree /repo__worktrees/feat-api --json

workmux add feat/api                 # now create the actual worktree
```

If the process dies mid-flight the slot is left as `unknown` — nothing is
auto-adopted or auto-released. Settle it explicitly:

```bash
workmux task resolve backend --retry     # put it back on the frontier
workmux task resolve backend --abandon   # give up; mark it failed
```

## Options

| Flag              | Default            | Notes                                     |
| ----------------- | ------------------ | ----------------------------------------- |
| `--graph <PATH>`  | `tasks/index.json` | relative paths resolve from the repo root |
| `--json`          | off                | machine-readable output (`list`, `get`)   |

## See also

- [`workmux add`](./add) — create the worktree a claimed task runs in
- [`workmux signal`](./signal) — how an agent reports progress back out of band
- [`workmux dashboard`](./dashboard) — the `Tasks` tab renders this graph
