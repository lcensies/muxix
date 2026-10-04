# muxix — agent instructions

Rust CLI that drives git worktrees and multiplexer windows (tmux, WezTerm, kitty,
Zellij) for parallel agent work.

## Scope boundary

muxix owns **deterministic plumbing**: worktree lifecycle, window/pane layout,
status tracking, signals, file sync, merge, sandboxes, agent setup.

It does **not** schedule work. There is no pipeline runner, no task loop, no
decomposition engine — an external harness decides what runs next and calls
muxix. The task graph (`muxix task` over `tasks/index.json`) is a store with
no scheduler attached; keep it that way. New "run this DAG for me" features
belong in the harness, not here.

## Build and check

```bash
just check        # format → clippy → unit tests → python lint → docs build
just unit-tests   # cargo test --bins
just test         # pytest end-to-end suite (needs tmux)
cargo check --all-targets
```

`just check-ci` fails when a check leaves the tree dirty, so run `just check` and
commit the result rather than fighting CI.

## Layout

| Path | What |
|---|---|
| `src/command/` | one module per CLI subcommand; `cli.rs` is the clap surface and dispatch |
| `src/workflow/` | worktree lifecycle shared by add/open/merge/remove |
| `src/multiplexer/` | tmux / WezTerm / kitty / Zellij backends behind one trait |
| `src/tasks/` | task graph store (atomic, locked writes) |
| `src/signals/` | agent↔muxix signals, hooks, and `muxix::event` tracing |
| `src/agent/` | agent profiles, registry, per-agent setup (`muxix setup`) |
| `src/command/sidebar/`, `src/command/dashboard/` | the two TUIs |
| `resources/` | files compiled into the binary with `include_str!` |
| `docs/` | VitePress site; `docs/reference/commands/` is the CLI reference |
| `tests/` | pytest end-to-end suite driving a real multiplexer |

## Conventions

- Tests live next to the code (`#[cfg(test)] mod tests`); end-to-end behaviour
  goes in `tests/` and drives the real binary, not mocks.
- A new CLI flag or command needs its `docs/reference/commands/*.md` page updated
  in the same change.
- `src/config.rs` holds both the config types and the annotated template
  `muxix init` writes — change them together.
- Events use the `muxix_evt!` macro so they land in the log file, never in a pane.
