---
description: Structured event tracing - what muxix logs, and how to filter it
---

# Event tracing

muxix emits a structured, greppable **event timeline** to the log file
(`$XDG_STATE_HOME/muxix/muxix.log`, never into a pane). It answers "did the
hook fire?", "which pane did the prompt go to?", "why did this command fail?"
without attaching to anything.

Implementation: [`src/signals/event.rs`](../../src/signals/event.rs) —
`wm_span!`, `wm_evt!`, `wm_evt_dbg!`. Everything uses the dedicated tracing
target `wm::event`, so it can be filtered or silenced wholesale.

```bash
grep 'ev=' ~/.local/state/muxix/muxix.log            # the whole timeline
grep 'ev="signal.write"' ~/.local/state/muxix/muxix.log  # one kind
grep 'pane=%12' ~/.local/state/muxix/muxix.log       # one pane
```

## Format

**JSON lines by default** — one object per line, both `jq`-queryable and
greppable. Each line carries `timestamp`, `level`, `target`, the event `fields`
(including `ev`), and the span context.

- `MUXIX_LOG_FORMAT=text` — switch to the human-readable formatter.
- `MUXIX_EVENTS=off` — silence the event target.
- `MUXIX_EVENTS=debug` — add the high-frequency `*.poll` / `*.retry` events.
- (equivalently `RUST_LOG=wm::event=off|debug`).

## Events

| Kind                    | Fields                              | Meaning                                                      |
| ----------------------- | ----------------------------------- | ------------------------------------------------------------ |
| `app.start`             | `args`, `version`                   | process started                                              |
| `app.done`              | —                                   | process exited cleanly                                       |
| `app.fail`              | `err`, `chain`                      | process exited with an error                                 |
| `cmd.dispatch`          | `cmd`                               | which subcommand was resolved                                |
| `config.load`           | `ok`, `err?`, `config_override?`    | config file resolved (or failed to)                          |
| `config.data` \*        | `cfg`                               | the fully merged config                                      |
| `signal.write`          | `kind`, `pane` or `node`, `side`    | a signal file was written (`side` = `agent` or `hook`)       |
| `hook.status`           | `status`, `pane`, `side`            | an agent status hook reported in                             |
| `agent.session`         | `hooks`, `agent`                    | `hooks-report`: which muxix hooks the session actually has |
| `pane.command.resolved` | `pane`, `cmd`                       | the command a pane was launched with                         |
| `prompt.file.written`   | `path`, `bytes`                     | a prompt file was staged for an agent                        |
| `prompt.sent`           | `bytes`                             | a prompt was delivered into a pane                           |
| `graph.lock.reclaim`    | `path`, `age_ms`                    | a stale task-graph lock was reclaimed                        |

\* `debug` level.

Because `muxix signal …` (the agent's hook subprocess) initialises the same
logger, its `signal.write` events interleave with the main process's events in
one file — so hook-write → observe latency is measurable from the timestamps.

## Configuring events

The env var controls the _level_ (all-or-nothing). To set that level from config,
and to silence _specific_ kinds or whole groups, use the `events:` section of
`.muxix.yaml`:

```yaml
events:
  enabled: true # master switch; false silences all wm::event. Default: true.
  level: debug # off | info | debug | trace. Ignored when MUXIX_EVENTS is set.
  disable: # silence these kinds/groups
    - config.data #   exact kind
    - pane #   group prefix: pane.command.resolved, ...
  only: [] # allowlist; non-empty => emit ONLY these (still minus `disable`)
```

Matching is **group-aware**: a pattern matches a kind when it equals the kind
exactly or is a dotted prefix of it — `pane` matches `pane.command.resolved` but
not `prompt.sent`.
