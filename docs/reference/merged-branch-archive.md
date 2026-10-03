---
description: Archive of merged feature branches and the functionality they introduced
---

# Merged branch archive

Record of feature branches that have been fully merged into `main` and whose
worktrees were removed on 2026-06-06. Each entry names the merge commit and the
modules/functionality the branch contributed, so the work stays discoverable
after the branches are deleted.

## orchestrator-preflight

- **Merge:** `044247b` (branch tip `572d1a5`)
- **Functionality:** Orchestrator runs setup-phase capabilities once as a
  preflight step before any task slots start.
- **Key files:** `src/pipeline/preflight.rs` (new, ~612 lines),
  `src/command/orchestrate.rs`, `src/pipeline/graph.rs`,
  `src/pipeline/{tui,types,mod}.rs`, `src/cli.rs`, `src/rpc/task_service.rs`.

## pipeline-node-phases

- **Merge:** `8861b00` (branch tip `600b243`)
- **Functionality:** Pipeline runner skips project-scoped setup nodes once the
  project has been provisioned (node phase tracking).
- **Key files:** `src/pipeline/project_state.rs` (new),
  `src/pipeline/runner.rs`, `src/command/pipeline.rs`,
  `src/pipeline/types.rs`, `src/cli.rs`.

## project-state-store

- **Merge:** `8562fdf` (branch tip `2905fc0`)
- **Functionality:** Per-project runtime state store holding capabilities and
  facts, with file locking.
- **Key files:** `src/project_state/` (new: `store.rs`, `types.rs`, `lock.rs`,
  `mod.rs`), `src/command/project_state.rs`, `src/command/mod.rs`,
  `src/main.rs`, `src/cli.rs`, `.gitignore`.

## scroll-in-planning-tab

- **Merge:** `6e1208c` (branch tip `c9d89aa`)
- **Functionality:** Native scrollback, mouse support, and modified-key handling
  in the dashboard planning tab.
- **Key files:** `src/command/dashboard/app/planning.rs`,
  `src/command/dashboard/ui/planning.rs`, `src/command/dashboard/keymap.rs`,
  `src/command/dashboard/{actions,mod}.rs`.

## Empty branches (no unique commits)

These branches never diverged from `main` (tip at `e71a07a`, 0 commits ahead) —
no functionality was lost when they were deleted:

- `auto-resume`
- `pipeline-yaml-includes`
- `penis`
