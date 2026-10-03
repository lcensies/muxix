---
description: Wait for one or more agents to reach a target status
---

# wait

Blocks until the named worktrees' agents reach a target status, or a timeout elapses. Used by coordinator skills and scripts to synchronize on agent progress instead of polling `status` in a loop.

```bash
workmux wait <name...> [flags]
```

## Arguments

- `<name...>`: one or more worktree names (supports cross-project `project:handle` syntax). Required.

## Options

| Flag               | Description                                                                                     |
| ------------------- | ------------------------------------------------------------------------------------------------- |
| `--status <list>`   | Comma-separated target(s) to wait for: `working`, `waiting`, `done`, `completed`, `failed`, `merged`. Default: `done`. |
| `--timeout <secs>`  | Maximum wait time in seconds. On timeout, prints the still-pending names and exits `1`.            |
| `--any`             | Return as soon as ANY named worktree reaches its target, instead of waiting for all of them.       |

## Status vs. completion

- `working` / `waiting` / `done` match the turn-boundary `AgentStatus` — the same status `workmux status` shows, driven by agent lifecycle hooks. `done` here means "agent finished a turn", not "task finished".
- `completed` / `failed` match an agent-authored *completion record*, written by [`workmux signal done`](./signal) / `workmux signal error` from inside the agent's own turn. Use these to wait on actual task completion rather than a turn boundary.
- `merged` matches "the agent was seen running, then its worktree disappeared" (i.e. `/merge` or equivalent ran). `completed` also treats a vanished worktree as a hit (historical merge-success fallback); `failed` does not — a vanished worktree is never treated as a failure.

When several requested targets could match at once (e.g. `--status completed,merged` after a merge), targets are checked in a fixed order — `working`, `waiting`, `done`, `completed`, `failed`, `merged` — and the first match wins.

## Crash handling

If an agent that was previously seen running disappears without hitting any requested target and its worktree still exists, `wait` reports that one target as exited (`<name>: agent exited unexpectedly`) and keeps waiting on the rest. It does not abort the whole invocation. Once every named worktree has either reached its target or exited, `wait` exits non-zero (`3`) if any exited; otherwise `0`.

::: warning Breaking change
Previously, one crashed agent aborted the entire `wait` immediately with exit code `3`. Scripts that relied on an immediate abort on the first crash must now check which names are missing from the successful set (or parse the `exited unexpectedly` lines) instead of assuming the whole invocation stopped at the first failure.
:::

## Examples

```bash
# Wait for a turn boundary (agent stopped responding)
workmux wait user-auth

# Wait for the agent to actually signal task completion
workmux wait user-auth --status completed

# Wait for either completion or failure, whichever comes first
workmux wait user-auth --status completed,failed

# Wait for several worktrees, returning as soon as any one finishes
workmux wait user-auth api-refactor --status completed --any

# Wait up to 10 minutes, otherwise give up
workmux wait user-auth --status completed --timeout 600

# Wait for a worktree that a /merge may have already removed
workmux wait user-auth --status merged
```

## Related

- [`workmux signal`](./signal) — how agents write the completion record `wait --status completed|failed` reads.
- [`workmux status`](./status) — inspect current status/completion without blocking.
