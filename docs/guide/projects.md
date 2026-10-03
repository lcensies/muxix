# Projects

Workmux can track multiple project directories and launch them all at once —
one tmux session per project, one window per worktree — turning it into a
single entry point for your whole agentic dev environment.

## Tracking projects

```bash
workmux project add ~/repos/my-app     # track a project
workmux project list                   # name + root, one per line
workmux project rm my-app              # untrack (by name or path)
```

The registry lives in `~/.config/workmux/projects.yaml` as a plain list and is
safe to hand-edit:

```yaml
- name: my-app
  root: /home/user/repos/my-app
```

As a shortcut, `workmux add <dir>` tracks the directory as a project whenever
the argument is an existing directory containing `.git` — otherwise it keeps
its usual meaning of creating a worktree.

## Opening one project

```bash
workmux project open my-app        # by name or path
workmux project open my-app -c     # ...and resume the last agent
```

Same session/layout/worktree-window setup as `workmux start`, scoped to one
project, then it focuses that session: switching the client when run inside
tmux, attaching when run from a plain shell.

## Starting everything

```bash
workmux start
```

For every tracked project:

- Ensures a tmux session named after the project exists (existing sessions are
  left untouched — the command is idempotent).
- Creates the session's base windows from a layout (see below).
- Opens one window per workmux worktree inside the project session, using the
  normal workmux window naming — so the dashboard, sidebar, `status`, and
  cross-project `project:handle` targeting all see them.

Projects whose directory no longer exists are skipped with a warning.

The multiplexing strategy is configurable via `project_mux` in the global
config; `session` (one session per project) is the default and currently the
only strategy.

## Base layouts

The base windows of a project session are resolved in priority order:

1. `windows:` in the project's `.workmux.yaml` (window names only)
2. `~/.config/tmuxrs/<name>.yml`
3. `~/.config/tmuxinator/<name>.yml`
4. A single shell window at the project root

For tmuxrs/tmuxinator files only simple `windows:` entries of the form
`- name: command` are honored; nested panes, ERB templating, and hooks are
ignored. This means existing tmuxinator projects keep working — just
`workmux project add` their roots.

## Resuming agents

```bash
workmux start -c   # --continue
```

Additionally relaunches the last coding agent in each project and worktree,
using the same resume ladder as [`workmux resurrect`](/reference/commands/):

1. **Resume** the journalled (or latest) conversation with the agent's own
   resume flag, verified against the agent's session store.
2. **Re-prompt**: no resumable session but a stored task prompt → the agent is
   relaunched fresh and the task is re-sent.
3. **Bare**: nothing to resume → the window stays a plain shell.

Agents without resume support are launched without a continue flag, with a
warning naming the agent.
