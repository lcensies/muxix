---
description: Emit an out-of-band agent signal, including agent-authored task completion
---

# signal

Emits an out-of-band signal from an agent's hooks, tools, or slash commands. Hidden command (not shown in `--help`); called by agent lifecycle hooks and by agents themselves, not typically run by hand.

```bash
muxix signal <kind> [flags]
```

## Arguments

- `<kind>`: one of `turn-done`, `needs-input`, `working`, `proceed`, `reject`, `done`, `error`.

## Options

| Flag                | Description                                                                                                                      |
| -------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `--pane <id>`        | Override the pane id (defaults to `$TMUX_PANE`). Used for pane-keyed signals, including `done`/`error` without `--node`.          |
| `--node <id>`        | Node id for agent-level signals (`done`/`error`). When set, writes the node-keyed pipeline hook-signal file instead of a pane-keyed completion. |
| `--feedback <text>`  | Feedback message. Used by `reject`, `error`, or `done` with feedback.                                                             |

## Two independent signal systems

**Pane-keyed turn signals** (`turn-done`, `needs-input`, `working`, `proceed`, `reject`) are keyed by `$TMUX_PANE` and drive agent lifecycle tracking (status hooks, `/implement`-style gates). Not covered further here — see [Status tracking](/guide/status-tracking).

**`done` and `error`** are dual-mode:

- **With `--node`**: node-keyed, writes the pipeline hook-signal file used for inter-stage messaging in the task-graph pipeline. Unchanged behavior.
- **Without `--node`**: pane-keyed, writes an agent-authored *completion record* onto the pane's `AgentState` (resolved from `$TMUX_PANE` or `--pane`). This is the signal [`muxix wait --status completed|failed`](./wait) and [`muxix status`](./status) read.

The pane-keyed form requires a resolvable pane: if neither `$TMUX_PANE` nor `--pane` nor `--node` is available, the command errors instead of guessing.

`muxix add` and `muxix send` clear any existing completion record on a pane before launching or delivering a new prompt, so a resurrected agent or a follow-up instruction cannot inherit a stale `completed`/`failed` from an earlier task.

## Examples

```bash
# Agent signals it finished its task (pane-keyed, no --node)
muxix signal done

# Agent signals it finished but wants to leave a note for the coordinator
muxix signal done --feedback "tests pass; left a TODO for the retry logic"

# Agent signals it hit an unrecoverable error
muxix signal error --feedback "migration script requires prod DB access I don't have"

# Node-keyed done, for pipeline stage transitions (unchanged, task-graph only)
muxix signal done --node build-stage --feedback "build artifacts uploaded"
```

## Related

- [`muxix wait`](./wait) — block until a pane's completion record (or turn status) reaches a target.
- [`muxix status`](./status) — show completion state alongside turn status.
- [Status tracking](/guide/status-tracking)
