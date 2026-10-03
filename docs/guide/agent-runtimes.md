# Agent runtimes

A *runtime* answers one question: **who owns the agent process**.

Workmux has always owned it — a git worktree, a multiplexer window, an agent
CLI, and status from hooks. That is now one runtime among others, called
`local`, and it is still the default. The alternative is to hand the agent to an
**agent development environment** (ADE): a manager that runs its own daemon and
its own desktop, web, and phone clients. Work started from workmux is then
reachable from your phone, without workmux growing a mobile stack.

```
workmux CLI/TUI
      │
      ▼
neutral agent-runtime API
      │
      ├─► local   → git worktree + multiplexer pane + agent CLI   (default)
      │
      └─► ADE     → that manager's daemon → Claude / Codex / …
                                  ▲
                                  │
                        its phone / web clients
```

## Not to be confused with

Workmux already has two things with adjacent names. A runtime is neither:

| | What it is |
| --- | --- |
| `AgentProfile` | one agent CLI's quirks — bang delay, prompt flag, skip-permissions flag |
| `AgentDefinition` | how a run is configured — model, permission mode, prompt, bootstrap |
| **`AgentRuntime`** | **who owns the process** |

The local runtime uses the other two internally, unchanged.

For the same reason a runtime declares **features**, not "capabilities":
"capability" already means an orchestrator setup unit (`preflight`, `cap_store`)
and the permissions an `AgentDefinition` grants a model.

## Listing what you have

```bash
workmux runtimes          # name, health, features
workmux agents            # every agent, whoever owns it
workmux agents --json
```

`workmux agents` is the runtime-shaped view — agents in your tmux panes next to
agents an ADE is running — while `workmux status` stays the worktree-shaped one.
Each row is prefixed with the runtime that owns it, and that prefix is the
reference other commands take:

```bash
workmux send paseo:agt_7c2 "run the tests again"
workmux agents stop paseo:agt_7c2
```

A bare handle or pane id still means a local agent, so nothing you type today
changes meaning.

An ADE whose CLI is missing shows as unavailable with the reason. The command
still succeeds — an unreachable manager is information, not a failure.

## Choosing one

```yaml
# .workmux.yaml
agent_runtime: paseo      # default: local
```

A task can override its project with a `runtime:<name>` label. Resolution order
is task override → project config → `local`.

Or per invocation:

```bash
workmux add fix-parser --runtime paseo -p "the parser drops trailing commas"
```

This setting decides who **creates** new agents. It does not decide who workmux
is allowed to talk to — see below.

An unknown or unhealthy runtime is an **error**. Workmux will not quietly run
the work locally: you chose a remote-reachable manager for its remote reach, and
losing that silently is exactly the failure you would notice too late.

## Ownership is exclusive; management is not

Exactly one process owns an agent. That is a fact about processes, not a policy:
the runtime named in an agent's reference is the one holding it.

Everything else is shared:

- **Workmux lists agents from every healthy runtime**, not just the configured
  one. Agents you started from your phone sit next to the ones in your tmux
  session.
- **Operations route by the agent, not by configuration.** `send` and `stop` go
  to whichever runtime owns that agent, so a project set to `local` can still
  drive an agent an ADE owns.
- **A local agent can be published into an ADE** with the `import` feature. The
  ADE adopts the underlying provider session for visibility and control, while
  the process stays in your tmux pane. Paseo does this with
  `paseo agent import <session> --provider <p> --cwd <path>`.
- **The sidebar and dashboard show every runtime's agents**, using one shared
  conversion so the two views cannot drift apart.
- **An unhealthy runtime is reported, never treated as owning nothing** — that
  would silently hide every agent it holds.

A pane-less agent still appears in those views, but anything pane-shaped —
preview, capture, switch-to-pane, freeze — is skipped for it rather than
attempted against a reference the multiplexer cannot resolve.

So "everything from Paseo, but managed in workmux" and the reverse are both the
intended shape, not a workaround.

## Features

| Feature | Meaning |
| --- | --- |
| `panes` | agents occupy a multiplexer pane that can be focused and captured |
| `owns-worktree` | the runtime creates and owns the git worktree |
| `send` | text can be sent to a running agent |
| `freeze` | agents can be suspended when idle |
| `fork` | a conversation can be forked into a new agent |
| `events` | status is pushed rather than polled |
| `import` | can adopt a session another runtime's process owns |

An operation a runtime does not declare fails naming both the runtime and the
feature. It never silently does nothing — a `send` that quietly vanishes because
the agent lives in another manager looks exactly like a delivered message.

ADE agents have no pane, so pane-shaped features (`panes`, `freeze`) are off:
the sidebar, freeze, and capture are local-runtime features.

## Configuring an ADE

An ADE is configuration, not code. Every argument list is a template, so
supporting another manager means another block of the same shape.

```yaml
# .workmux.yaml
ade:
  paseo:
    command: paseo
    start_args: [agent, run, --json, --cwd, "{cwd}", "{prompt}"]
    send_args:  [agent, send, "{id}", "{text}"]
    list_args:  [agent, ls, --json]
    stop_args:  [agent, stop, "{id}"]
    id_field: id
    status_field: status
    status_map:
      working: [running, thinking]
      waiting: [waiting, needs_input]
      done:    [closed, completed]
      failed:  [error]
```

Statuses map onto one neutral set: `starting`, `working`, `waiting`, `done`,
`failed`, `unknown`. `failed` is separate from `done` on purpose — a manager
reporting an error must never render as successful completion. Paseo's real
lifecycle states (`initializing`, `idle`, `running`, `error`, `closed`) are
covered by the built-in preset.

Paseo ships as a built-in preset; the block above only tweaks it. A state the
map does not cover becomes `unknown` rather than a guess.

Workmux drives ADEs through their CLI rather than their wire protocol. That
keeps zero protocol code here and survives their schema churn, at the cost of
polled status instead of pushed events — the same trade workmux already makes
with tmux and git. A streaming implementation can replace it behind the same
trait without touching callers.

## Project sync

ADEs usually track projects too, so workmux can keep its registry in agreement
with theirs. Workmux is not privileged here: it implements the same
`ProjectRegistrySource` interface an ADE does, because it has project
management, a daemon, and agent management — which is what makes something an
ADE in the first place.

```yaml
ade:
  paseo:
    projects:
      list_args:   [project, ls, --json]
      add_args:    [project, create, "{name}", "{root}"]
      remove_args: [project, delete, "{name}"]
      sync:
        direction: bidirectional   # off (default) | pull | push | bidirectional
        conflict: manual           # manual (default) | workmux | ade | newest
        removals: false            # default
        names: false               # default
```

```bash
workmux project sync --dry-run     # show adds, removes, conflicts
workmux project sync --ade paseo
```

Sync also runs on the system daemon's tick, rate-limited, and skipped entirely
when every ADE has `direction: off`.

### Why the defaults are timid

- **Identity is the canonical root path, never the name.** Both tools let you
  name a project whatever you like; the same directory under two names is one
  project, not a conflict.
- **Removals are off by default,** and even when on, a removal is only inferred
  from a recorded last-synced set. "Absent over there" alone cannot be told
  apart from "newly added over here" — that ambiguity is how naive bidirectional
  sync deletes things, or ping-pongs forever re-adding them.
- **Renames are off by default.** Two tools may legitimately call the same
  directory different things.

Enabling removals never retroactively deletes: projects that predate the setting
have no last-synced record, so they read as additions.
