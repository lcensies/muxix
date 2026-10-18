---
description: Control the muxix daemon that serves the sidebar and runs background jobs
---

# daemon

One daemon per tmux server. It watches tmux and agent state, pushes snapshots to
[sidebar](./sidebar.md) panes, and runs configured background jobs — today, ADE
[project sync](../../guide/agent-runtimes.md#project-sync).

```bash
muxix daemon start    # start if not running (idempotent)
muxix daemon status   # pid, and which background jobs are configured
muxix daemon stop     # SIGTERM, even if jobs are configured
```

## Lifetime

The daemon exits on `muxix daemon stop`, on SIGTERM, and when its tmux server
is gone.

With **no background job configured** it also exits ten seconds after the last
sidebar pane closes, and `muxix sidebar` toggle-off stops it — the sidebar is
its only consumer, so nothing changes for sidebar-only setups.

With a job configured (any ADE whose `projects.sync.direction` is not `off`) the
daemon stays up without a sidebar, and toggling the sidebar off leaves it
running.

## Starting

`muxix sidebar` always starts it. When a background job is configured, so do
`muxix add`, `muxix open`, and the agent status hooks, so the daemon comes up
with the first agent of the day. Each of those is a socket connect; a process is
spawned only when the connect fails.

## What runs inside

- tmux state, agent status, stall detection, auto-freeze (`daemon.auto_freeze`)
- git status and PR workers, config file watcher
- ADE agent listing (`ade.*`)
- ADE project sync, once per ADE per `projects.sync.interval` (default `1m`)

Logs go to `~/.local/state/muxix/muxix.log`; sync results appear as
`project sync applied` / `project sync failed` lines.

Internals: the daemon is the hidden `muxix _daemon` subcommand, its PID is the
tmux global option `@muxix_daemon_pid`, and its socket is
`$TMPDIR/muxix-daemon-<instance>.sock`.
