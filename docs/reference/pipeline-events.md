# Pipeline event tracing

The pipeline runner emits a structured, greppable **event timeline** to the
normal log file (`~/.local/state/workmux.log`, never the pane). It exists to
answer "why was the prompt slow?" / "why didn't `implement` hand off to `test`?"
by reconstructing the out-of-band dance between the runner, the tmux pane, and
the agent's hook subprocess.

Implementation: [`src/pipeline/event.rs`](../../src/pipeline/event.rs)
(`wm_span!`, `wm_evt!`, `wm_evt_dbg!`), wired through `src/pipeline/runner.rs`
and `src/command/signal.rs`.

## Model: spans carry context, events mark transitions

Ambient context lives on **spans** (one per function/scope); discrete
transitions are **events**. Because the subscriber prints the active span scope
on every line, every event automatically carries its enclosing context — events
themselves stay lean.

| Span         | Fields                  | Scope                       |
| ------------ | ----------------------- | --------------------------- |
| `workflow`   | `pane`                  | a whole `Runner::run`       |
| `node`       | `node`, `kind`, `phase` | one node's execution        |
| `swap`       | `pane`                  | agent (re)launch for a node |
| `pane_ready` | `pane`                  | readiness gate + echo probe |
| `turn`       | `pane`                  | one prompt→completion turn  |

`kind` ∈ `agent | bash | loop | breakpoint | signal | decompose | none`.

## Format

**JSON lines by default** — one object per line, both `jq`-queryable and
greppable. Each line carries `timestamp`, `level`, `target`, the event `fields`
(including `ev`), and the `span`/`spans` context.

- `WORKMUX_LOG_FORMAT=text` — switch to the human-readable formatter.
- `WORKMUX_EVENTS=off` — silence the event target in production.
- `WORKMUX_EVENTS=debug` — add the high-frequency `*.poll` / `*.retry` / `node.blocked` events.
- (equivalently `RUST_LOG=wm::event=off|debug`).

## Configuring events (`.workmux.yaml`)

The env var controls the _level_ (all-or-nothing per level). To set that level
from config, and to silence _specific_ kinds or whole groups, use the `events:`
section:

```yaml
events:
  enabled: true # master switch; false silences all wm::event. Default: true.
  level: debug # off | info | debug | trace. Ignored when WORKMUX_EVENTS is set.
  disable: # silence these kinds/groups
    - pane.probe #   group prefix: pane.probe.ok, pane.probe.retry, ...
    - turn.poll #   exact kind
  only: [] # allowlist; non-empty => emit ONLY these (still minus `disable`)
```

Matching is **group-aware**: a pattern matches a kind when it equals the kind
exactly or is a dotted prefix of it — `pane.probe` matches `pane.probe.ok` but
not `pane.gate.ok`.

**Precedence:**

1. `enabled: false` wins over everything — no `wm::event` output at all.
2. Level: `WORKMUX_EVENTS` env (if set) → else `events.level` → else `info`.
3. Per-kind: `only` (allowlist, if non-empty) then `disable` (denylist), both
   group-aware, applied on top of whatever the level lets through.

In a project + global config, `events` merges per-field (project overrides
global; `only`/`disable` use the project list when it sets any). Note: the
earliest events (`app.start`, `config.load`) fire before config is read, so
per-kind filtering applies from config-load onward.

## Event catalogue

`*` = `debug` level (only with `WORKMUX_EVENTS=debug`). All others are `info`.

### Process lifecycle (every `workmux` invocation)

| `ev`             | fields                           | meaning                                                                                     |
| ---------------- | -------------------------------- | ------------------------------------------------------------------------------------------- |
| `app.start`      | `version`, `args`, `cwd`, `pane` | binary started (emitted by _every_ invocation, incl. agent hook subprocesses)               |
| `app.done`       | —                                | clean exit                                                                                  |
| `app.fail`       | `err`, `chain`                   | exit with error; `chain` is the full anyhow cause chain (Debug) — the source of the failure |
| `cmd.dispatch`   | `cmd`                            | the parsed subcommand + its args (`?Commands` Debug)                                        |
| `config.load`    | `ok`, `config_override`, `err?`  | config resolved (or failed)                                                                 |
| `config.data` \* | `cfg`                            | the full resolved config (Debug)                                                            |

### Orchestrator (multi-task graph loop)

| `ev`                  | fields                                                    | meaning                                |
| --------------------- | --------------------------------------------------------- | -------------------------------------- |
| `orch.start`          | `graph`, `slots`, `base`, `port`, `dry_run`, `auto_merge` | orchestrator loop started              |
| `orch.tick` \*        | `tasks`, `active_slots`                                   | one scheduler tick                     |
| `task.claim`          | `task`, `title`                                           | task claimed → in_progress             |
| `task.claim.fail`     | `task`, `err`                                             | claim (status write) failed            |
| `task.spawn`          | `task`, `worktree`                                        | harness spawned in a new worktree/pane |
| `task.spawn.fail`     | `task`, `err`                                             | spawn failed (task reset to todo)      |
| `task.resume`         | `task`, `worktree`                                        | orphaned in_progress task re-attached  |
| `task.resume.fail`    | `task`, `err`                                             | resume failed                          |
| `task.merge`          | `task`, `worktree`                                        | merge of a completed task began        |
| `task.done`           | `task`, `via`                                             | task merged + marked done              |
| `task.merge.conflict` | `task`, `on_conflict`                                     | merge conflict (→ agent or manual)     |
| `task.merge.fail`     | `task`, `err`                                             | merge errored → task failed            |

### Workflow / DAG (per-task pipeline runner)

| `ev`              | fields                    | meaning                                                |
| ----------------- | ------------------------- | ------------------------------------------------------ |
| `workflow.start`  | `nodes`, `pane`           | scheduler started                                      |
| `workflow.done`   | `elapsed_ms`              | all nodes resolved                                     |
| `workflow.stuck`  | `elapsed_ms`              | no progress: unmet deps / cycle                        |
| `node.run`        | — (span: node)            | node execution began                                   |
| `node.done`       | `elapsed_ms`              | node succeeded                                         |
| `node.fail`       | `elapsed_ms`, `err`       | node failed                                            |
| `node.skip`       | `node`, `reason`, `when?` | skipped (setup-satisfied / dep-failed / when-mismatch) |
| `node.fallback`   | `node`                    | `on_failure` fallback node scheduled                   |
| `node.blocked` \* | `node`, `deps`            | not yet eligible (deps pending)                        |

### Agent swap (per-node (re)launch)

| `ev`            | fields         | meaning                                         |
| --------------- | -------------- | ----------------------------------------------- |
| `agent.swap`    | `plan`, `perm` | agent relaunched in the pane (`respawn-pane`)   |
| `agent.restore` | `from`         | restored from a snapshot instead                |
| `signal.clear`  | `kind`         | cleared a stale marker before relaunch          |
| `swap.skip` \*  | `reason`       | no relaunch needed (fresh-noop / not-agent / …) |

### Pane readiness (the prime "slow prompt" suspect)

| `ev`                      | fields                          | meaning                                        |
| ------------------------- | ------------------------------- | ---------------------------------------------- |
| `pane.gate.wait`          | —                               | waiting for `session-ready` (Layer 1)          |
| `pane.gate.ok`            | `how`, `elapsed_ms`             | gate passed (`how=hook` or `content-fallback`) |
| `pane.gate.timeout`       | `elapsed_ms`                    | gate never resolved                            |
| `pane.probe.ok`           | `attempts`, `elapsed_ms`        | echo probe confirmed the input box (Layer 2)   |
| `pane.probe.inconclusive` | `attempts`, `elapsed_ms`        | probe gave up after 20s, sent anyway           |
| `pane.probe.timeout`      | `attempts`, `elapsed_ms`        | probe hit the node deadline                    |
| `pane.probe.retry` \*     | `attempts`, `echoed`, `cleared` | one failed probe round                         |

> `how=content-fallback` with a large `elapsed_ms` (~15s) means **no
> `session-ready` hook fired** — the gate burned the full grace window. That is
> the usual cause of "prompts sent very slowly".

### Prompt + turn

| `ev`               | fields                                                   | meaning                                          |
| ------------------ | -------------------------------------------------------- | ------------------------------------------------ |
| `prompt.send`      | `bytes`, `multiline`                                     | about to type the prompt into the pane           |
| `prompt.sent`      | —                                                        | prompt + Enter committed; turn clock starts      |
| `turn.needs_input` | —                                                        | agent blocked on a user question; timeout paused |
| `turn.resumed`     | —                                                        | user answered; turn resumed                      |
| `turn.done`        | `reason`, `elapsed_ms`, `signal_age_ms?`                 | turn complete (`reason=signal` or `idle`)        |
| `turn.timeout`     | `timeout_s`, `elapsed_ms`                                | turn exceeded node timeout                       |
| `turn.poll` \*     | `turn_done`, `needs_input`, `seen_working`, `idle_polls` | one 500ms poll                                   |

> `signal_age_ms` is how old the `turn-done` file was when the runner noticed
> it. **Large `signal_age_ms`** → the hook fired long ago, the runner was slow
> to poll/detect (detection lag). **Small** → the hook itself was late (compare
> with the `signal.write` line). `reason=idle` means the Stop hook never fired
> and the content fallback declared completion (≥3s after work stopped).

### Gates (human / agent approval)

| `ev`           | fields                             | meaning                                                                 |
| -------------- | ---------------------------------- | ----------------------------------------------------------------------- |
| `gate.wait`    | —                                  | breakpoint waiting for approve/reject                                   |
| `gate.fired`   | `source`, `approved`, `elapsed_ms` | decision arrived (`source=node-key` TUI/CLI or `pane-key` `/implement`) |
| `gate.retry`   | `retry_node`                       | rejection reset `retry_node` + downstream                               |
| `signal.wait`  | —                                  | unified-signal gate waiting                                             |
| `signal.fired` | `approved`, `elapsed_ms`           | unified-signal gate resolved                                            |

### Signal write side (the agent hook subprocess)

| `ev`           | fields                        | meaning                                        |
| -------------- | ----------------------------- | ---------------------------------------------- |
| `signal.write` | `kind`, `pane`/`node`, `side` | a `workmux signal …` invocation wrote a marker |

`side=hook` (pane lifecycle: turn-done/session-ready/needs-input/proceed) or
`side=agent` (node-keyed done/error). This is the **other end** of the latency:
its timestamp vs the runner's `turn.done` / `gate.fired` is the true end-to-end
signal delay.

## Recipes

```sh
# Default is JSON; for ad-hoc runs you can switch to text:
WORKMUX_LOG_FORMAT=text workmux pipeline run -f wf.yaml

LOG=~/.local/state/workmux.log

# Whole timeline for one task's run (JSON):
jq -c 'select(.target=="wm::event")' "$LOG"

# How long each turn took, and how it completed:
jq -c 'select(.fields.ev=="turn.done")
       | {node: (.spans[]|select(.node).node), reason:.fields.reason,
          dt:.fields.elapsed_ms, age:.fields.signal_age_ms}' "$LOG"

# Where did the per-node wall-clock go? (slowest first)
jq -c 'select(.fields.ev=="node.done") | {node:.fields.node, ms:.fields.elapsed_ms}' "$LOG"

# Did the session-ready hook ever fire, or did we burn the 15s gate grace?
jq -c 'select(.fields.ev=="pane.gate.ok") | {how:.fields.how, ms:.fields.elapsed_ms}' "$LOG"

# End-to-end turn-done latency: pair the hook write with the runner observe.
grep -E 'ev":"(signal.write|turn.done)"' "$LOG"
```
