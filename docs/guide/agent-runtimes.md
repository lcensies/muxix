---
description: Who owns an agent process — muxix's own worktrees and panes, or an external agent manager whose sessions muxix can view, take over, and hand back
---

# Agent runtimes

A *runtime* answers one question: **who owns the agent process**.

Muxix has always owned it — a git worktree, a multiplexer window, an agent
CLI, and status from hooks. That is now one runtime among others, called
`local`, and it is still the default. The alternative is to hand the agent to an
**agent development environment** (ADE): a manager that runs its own daemon and
its own desktop, web, and phone clients. Work started from muxix is then
reachable from your phone, without muxix growing a mobile stack.

```
muxix CLI/TUI
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

Muxix already has two things with adjacent names. A runtime is neither:

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
muxix runtimes          # name, health, features
muxix agents            # every agent, whoever owns it
muxix agents --json
```

`muxix agents` is the runtime-shaped view — agents in your tmux panes next to
agents an ADE is running — while `muxix status` stays the worktree-shaped one.
Each row is prefixed with the runtime that owns it, and that prefix is the
reference other commands take:

```bash
muxix send paseo:agt_7c2 "run the tests again"
muxix agents stop paseo:agt_7c2
```

A bare handle or pane id still means a local agent, so nothing you type today
changes meaning.

An ADE whose CLI is missing shows as unavailable with the reason. The command
still succeeds — an unreachable manager is information, not a failure.

## Choosing one

```yaml
# .muxix.yaml
agent_runtime: paseo      # default: local
```

A task can override its project with a `runtime:<name>` label. Resolution order
is task override → project config → `local`.

Or per invocation:

```bash
muxix add fix-parser --runtime paseo -p "the parser drops trailing commas"
```

This setting decides who **creates** new agents. It does not decide who muxix
is allowed to talk to — see below.

An unknown or unhealthy runtime is an **error**. Muxix will not quietly run
the work locally: you chose a remote-reachable manager for its remote reach, and
losing that silently is exactly the failure you would notice too late.

## Ownership is exclusive; management is not

Exactly one process owns an agent. That is a fact about processes, not a policy:
the runtime named in an agent's reference is the one holding it.

Everything else is shared:

- **Muxix lists agents from every healthy runtime**, not just the configured
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

So "everything from Paseo, but managed in muxix" and the reverse are both the
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
# .muxix.yaml
ade:
  paseo:
    command: paseo
    start_args: [agent, run, --json, --workspace, "{workspace}", --cwd, "{cwd}", "{prompt}"]
    send_args:  [agent, send, "{id}", "{text}"]
    list_args:  [agent, ls, --json]
    stop_args:  [agent, stop, "{id}"]
    import_args: [agent, import, "{session}", --provider, "{provider}", --cwd, "{cwd}", --json]
    import_providers: [claude, codex, copilot, opencode, pi, omp]
    id_field: id
    status_field: status
    status_map:
      working: [running, thinking]
      waiting: [waiting, needs_input]
      done:    [idle, completed]
      exited:  [closed]
      failed:  [error]
```

Statuses map onto one neutral set: `starting`, `working`, `waiting`, `done`,
`exited`, `failed`, `unknown`. `failed` is separate from `done` on purpose — a manager
reporting an error must never render as successful completion. Paseo's real
lifecycle states (`initializing`, `idle`, `running`, `error`, `closed`) are
covered by the built-in preset. Paseo has no needs-input state: `idle` is a
finished turn, so it maps to `done`, and `closed` (process gone, session still
resumable) to `exited`.

Paseo ships as a built-in preset; the block above only tweaks it. A state the
map does not cover becomes `unknown` rather than a guess.

Muxix drives ADEs through their CLI rather than their wire protocol. That
keeps zero protocol code here and survives their schema churn, at the cost of
polled status instead of pushed events — the same trade muxix already makes
with tmux and git. A streaming implementation can replace it behind the same
trait without touching callers.

## Workspaces

An ADE's clients navigate by **workspace**, not by directory. Paseo's phone and
web clients resolve an agent's workspace id against the host's workspace list,
and an id that misses renders "Workspace unavailable" instead of the session. A
worktree muxix just created is exactly the path no ADE has seen.

So before handing over a worktree **muxix owns**, muxix resolves a workspace for
it and passes the id to `start_args` as `{workspace}`:

```yaml
ade:
  paseo:
    workspaces:
      list_args: [workspace, ls, --json]
      add_args:  [workspace, create, --isolation, local, --path, "{root}", --title, "{name}", --json]
      id_field:   workspaceId
      root_field: cwd
      ensure:     true   # default
```

Resolution lists first and matches the directory by canonical path, creating a
workspace only on a miss. `paseo workspace create` returns a *new* workspace for
a path it already tracks, so a create-per-handoff would pile up duplicates.

Three cases leave this alone:

- **The ADE made the worktree.** With `owns_worktree`, muxix passes no worktree
  of its own and the ADE has already registered whatever it created — Paseo's
  `--new-workspace worktree`, Orca's equivalent. Nothing is resolved.
- **The ADE has no `workspaces` block,** or sets `ensure: false`. Starting an
  agent then issues exactly one command, as before.
- **Resolution fails.** The agent still starts; muxix warns with the directory
  and the ADE's error. An unregistered agent runs fine, it just may not open in
  the ADE's own clients — information, not a reason to refuse the work.

`import` runs the same step for the session's directory. `paseo agent import`
takes no workspace flag and resolves one from the path, so the workspace has to
exist before the import rather than be named in it.

This block is a stopgap shaped to what paseo 0.11 exposes. An ADE that grows
first-class "adopt this worktree" support wants that instead; drop the block or
set `ensure: false` when it does.

## Viewing and driving an ADE session

An ADE agent has no pane — `features().panes` is false — so there is nothing for
muxix to switch to. Selecting one instead opens muxix's own read-only view of
the session, built from the ADE's client-side commands:

```yaml
ade:
  paseo:
    view:
      state_args:   [inspect, "{id}", --json]
      log_args:     [agent, logs, "{id}", -f]
      permit_args:  [permit, allow, "{id}"]
      deny_args:    [permit, deny, "{id}"]
      mode_args:    [agent, mode, "{id}", "{mode}"]
      desktop_args: [agent, open, "{id}"]
      handle_file:  "~/.paseo/agents/{cwd_slug}/{id}.json"
      handle_field: persistence.nativeHandle
      on_select:    tui   # default; also `ade` or `desktop`
      placement:    project   # default; also `window` or `split`
      takeover_placement: project
      window_name:  "{runtime}-{id}"
      follow_reconnect: true     # default
      follow_reconnect_delay_ms: 500
      follow_reconnect_max_delay_ms: 10000
      on_takeover_exit: exited   # default; or `manager`
```

`state_args` renders the view's chrome — status, mode, model, pending
permissions. `log_args` output is embedded verbatim as the body: paseo's
`logs --json` ignores the flag and emits the same human text either way, so
muxix passes it through rather than scraping an undocumented format.

**The view claims nothing.** Every entry above is a stateless call to the ADE's
daemon, which is the only writer to a session — it owns the provider process's
stdin, and each client asks it to write. So a muxix view is a peer of the phone
and desktop clients, and all three can use one session at the same time. This is
why `logs -f` is used rather than `attach`: attach claims the invoking
terminal's stdin, which a muxix-driven pane needs for itself.

`on_select` changes what selecting does: `tui` opens the muxix view, `ade` runs
`log_args` in a plain pane, `desktop` runs `desktop_args` and opens no pane. An
ADE with no `view` block reports that its sessions cannot be opened, rather than
doing nothing.

### Where a session's pane opens

`placement` decides the pane's host, and the default mirrors the shape `muxix
open` gives a project — a tmux session per project, a window per agent:

| Value | Where the pane lands |
|---|---|
| `project` | One window per agent, in the tmux session named for the project owning the session's `cwd`. The session is created if missing. Default. |
| `window` | One window per agent in the session that is already current, without resolving a project. |
| `split` | Beside the current pane. The only placement every backend can honour. |

The project comes from the **session's** working directory as the ADE reports
it, not from the directory muxix was invoked in. That is what keeps another
project's session out of this project's windows.

`window_name` names the per-agent window (`{id}`, `{runtime}`). A template that
renders blank lets the backend auto-name instead of creating a window no lookup
can find again.

`takeover_placement` is separate on purpose: a native takeover starts a provider
process that owns a transcript, and closing a read-only view must not end it.

Placement is advisory. A backend without session support, or a `cwd` belonging
to no registered project, falls back — `project` → `window` → `split` — because
where the session sits matters less than being able to open it at all.

### Surviving the manager's own timeouts

The manager's log client is not a durable stream. paseo's gives up on its own
daemon and exits:

```
Warning: failed to fetch existing timeline DaemonConnectionError: Timeout waiting for message (2000ms)
  code: 'DAEMON_REQUEST_TIMEOUT'
```

That timeout is **not reachable from outside**: `paseo agent logs` has no flag
for it, and nothing in the shipped client reads an environment variable for it
(the only `PASEO_*TIMEOUT*` vars are for provider refresh and shell env). So the
view does not try to ask for a longer one — it treats the exit as expected and
reconnects:

| Key | Default | Meaning |
|---|---|---|
| `follow_reconnect` | `true` | Respawn the log command when it exits. |
| `follow_reconnect_delay_ms` | `500` | First delay before reconnecting. |
| `follow_reconnect_max_delay_ms` | `10000` | Ceiling for the doubling delay. |

The delay doubles on each consecutive failure up to the ceiling, and a stream
that delivered output resets it — that is the difference between one flaky
request and a daemon that is not coming back. Each attempt appends a line to the
body (`-- stream ended; reconnecting in 500ms --`), so a reconnecting view never
looks like a live one.

With `follow_reconnect: false` the view says the stream ended and stops, which
is the old behavior. Closing the view kills the stream command, so the manager's
client does not outlive the pane.

### Taking a session over natively

A session is also a provider session on disk, so it can be resumed in the
provider's own CLI — `claude --resume <handle>` — giving a native TUI instead of
a view. `handle_file` plus `handle_field` say where to read that handle;
`handle_args` takes precedence when an ADE reports it through its CLI instead.

Takeover is opt-in because it is the one path that adds a **second** writer to an
append-only transcript. It serializes writers for that session and hands the
session back when the local agent exits, so any device can use it afterwards.
Note that an `idle` agent is not an unattended one: the provider process stays
resident between turns, so idle describes the turn, not the absence of a writer.

`muxix agents takeover` refuses while the manager may still hold a writer —
every state but `exited` — unless `--stop-ade` is given; `E` in a view does that
step itself. Stopping interrupts a turn in flight, so take over an **idle**
session: paseo's `agent stop` is a no-op there and nothing is lost. A `working`
one gets its turn cut off.

Pressing `E` in a view takes over **in that pane**, and the session is bound to
it: quitting the provider, or its crashing, ends the takeover. Because the
manager keeps reporting `idle` throughout, muxix tracks the takeover itself.
While it runs, the session shows the provider's live status; once it is quit,
`view.on_takeover_exit` decides what it shows:

| Value | After a takeover is quit |
|---|---|
| `exited` | `exited`, until the manager reports a turn underway (working, or waiting on a prompt) — one sent from another device — then the manager's status. Default. |
| `manager` | The manager's own status straight away. |

### Deleting a session

`delete_args` lets muxix delete an agent in the manager when you delete it from
the sidebar (`d`). The paseo preset uses `[agent, delete, "{id}"]`, a hard
delete. The manager is asked first and muxix forgets its own record only once
that succeeds; a manager with no `delete_args` keeps its copy. Muxix refuses
to render a blank `{id}` into this template, since paseo treats the id as an
optional prefix and a blank one would match every agent it has.

## Project sync

ADEs usually track projects too, so muxix can keep its registry in agreement
with theirs. Muxix is not privileged here: it implements the same
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
        conflict: manual           # manual (default) | muxix | ade | newest
        removals: false            # default
        names: false               # default
```

```bash
muxix project sync --dry-run     # show adds, removes, conflicts
muxix project sync --ade paseo
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
