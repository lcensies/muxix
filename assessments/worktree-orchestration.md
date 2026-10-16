# Worktree orchestration: skill vs command, and the recommended interface

Verdict doc. Question asked: is the `worktree-workflow` skill the right surface for
"orchestrator auto-creates a worktree and starts implementation there", or should
muxix grow a `muxix delegate` subcommand?

## 1. Verdict: keep the skill, do not add a subcommand

**Skill: keep, but understand who it is for.** `worktree-workflow` is a bash recipe
book. A coordinator-mode orchestrator has no shell, so every line in it needs a
taskflow child to execute. The skill is genuinely useful *to executor-tier
children*; it is close to useless to the orchestrator directly. That is a
mis-targeting, not a defect — do not rewrite the skill to fix it.

**`muxix delegate`: do not build.** Two independent reasons.

1. **It already exists, spelled differently.** `muxix add <branch> -b -P <promptfile>`
   is the dispatch primitive: worktree + branch + window + spawned agent + prompt.
   `muxix task create` / `claim` give the binding, `wait` / `capture` / `send` give
   control, `merge` / `remove` give cleanup. A `delegate` command collapses three
   existing commands into one. Thin.
2. **It becomes a scope violation the moment it does more.** `CLAUDE.md` is explicit:
   muxix owns deterministic plumbing and **does not schedule**. "Spin up an agent
   and etc" — where the *etc* is retry, verify, pick-the-next-step — is a scheduler,
   and schedulers belong in the harness (taskflow), not in muxix.

The one argument that could justify the Rust command later is **atomicity**: making
`task create` + `task claim` + `add` a single operation with rollback, so a crash
cannot leave an untracked tree or an orphaned binding. That is real plumbing value,
but YAGNI until the failure actually bites. Revisit only with a concrete incident.

## 2. Recommended interface: a saved taskflow, not a muxix subcommand

Name: **`/tf:wt-implement`** — a saved taskflow (`taskflow action=save`), registered
as a slash command at session start.

Division of labour, which is the whole point:

- **muxix owns the workspace lifecycle** — worktree, branch, window, status signals,
  merge, cleanup, resurrect. Deterministic plumbing, exactly its declared scope.
- **taskflow owns the graph** — which phases run, their dependencies, the verify
  gate, retries, resumability. Scheduling, exactly the harness's scope.

Exact invocation, as two orchestrator calls:

```
# call 1 — create the workspace, learn its path (taskflow `script` phase, zero tokens)
muxix add <branch> --name <handle> -b -C -F && muxix path <handle>

# call 2 — run the graph in that tree, with the path as a LITERAL cwd
taskflow action=run name=wt-implement args={"cwd":"<path from call 1>","task":"..."}
```

### The branch is not a taskflow argument

`muxix add` checks the branch out *inside* the worktree. A phase with
`cwd=<worktree path>` is therefore already on that branch. Taskflow needs the
**path only** — never a branch name, never a ref. Branch, merge and deletion stay
entirely on the muxix side, addressed by handle.

### Do not use `cwd: "worktree"` here

The taskflow `cwd: "worktree"` keyword runs its own
`git worktree add -b tf/<run>/<phase> HEAD` and, at phase teardown, **removes the
worktree and deletes the branch**. Inside a muxix worktree that is double
isolation: a nested throwaway tree off the muxix branch's HEAD, destroyed when the
phase ends, work gone unless it was pushed elsewhere. Use a literal `cwd`.

`cwd: "worktree"` remains correct for its actual purpose: throwaway experiments and
tournament variants, where deleting the branch *is* the desired outcome.

### Why `-C -F`, and why no pane agent

- `-b` background: the orchestrator stays where it is.
- `-C` plain shells instead of pane commands: **no agent is spawned in the pane**.
- `-F` skip file copy/symlink ops: no hook noise.

The result is a plain, persistent, mergeable checkout. Taskflow's executor phase
*is* the agent, so the coordinator keeps ownership of its children, gains a verify
gate, and gets a resumable DAG. Spawning an interactive agent that then runs a
taskflow would be two layers of indirection for the same work.

## 3. Orchestrator flow, by phase

| # | Phase | What happens |
|---|---|---|
| 0 | **Scout** | One parallel round of 2-4 cheap locational questions (paths, symbols, line numbers, existing tests). Read-only. Feeds the plan; does not replace reading the files about to be edited. |
| 1 | **Plan** | Decided in the orchestrator. Decisions only the user can make go to the user explicitly — never guessed. |
| 2 | **Worktree create** | `script` phase: create the tree and capture its path. Persistent branch, no agent, no hooks. |
| 3 | **Agent spin-up** | `agent` phase, `agent: executor`, literal `cwd` = the path from phase 2. This is the implementation step. No muxix command involved — taskflow is the agent. |
| 4 | **Verify** | `gate` phase in the same `cwd`, running the project's own check command, emitting `VERDICT: PASS` / `BLOCK`. A blocked gate stops the flow before anything reaches the main tree. |
| 5 | **Inspect** | Report the diff and the gate verdict back to the user. Review happens **before** any merge. |
| 6 | **Cleanup** | Merge on the user's explicit OK, or drop the tree. Never merge unreviewed; merge serially, one confirmed before the next starts. |

### Opt-in variant: `-P` dispatch to a real pane agent

Use only when the user asks for it, or when per-task dispatch is the declared mode.
It is the right choice for long-running or interactive work the user intends to
watch and steer — and it means the orchestrator **hands ownership away**: no verify
gate, no DAG, an independent agent.

Rules that path carries:

- Record the task **before** the tree exists, so a crash leaves a recoverable
  binding instead of an untracked tree. Claim before create; a task already bound
  elsewhere fails and changes nothing — read the existing binding rather than
  picking a name around it.
- Prompts are self-contained: the child cannot see the orchestrator's conversation.
  Use relative paths, and delegate a skill by name rather than restating its steps.
- Write every prompt file first, then run every `muxix add`; always `-b`.
- Always pass `--timeout` to `wait`.
- With `--fork` the child inherits the conversation, so prepend an anti-recursion
  note: *you are inside a worktree created by dispatch; do not dispatch further
  worktrees.*
- The prompt arrives as argv (`pi [args] -- "$(cat PROMPT.md)"`), i.e. the file's
  **contents** become the first user message. Config `prompt_file_only` flips this
  to pass the path instead — leave it off for pi.

## 4. Step-to-command map

| Step | muxix commands | Notes |
|---|---|---|
| Worktree create | `muxix add <branch> --name <handle> -b -C -F` | `--pr <n>` for a pull request; `--base <ref>` to branch from something other than the current branch; `-o` to open an existing worktree instead of failing. |
| Resolve path (feeds taskflow `cwd`) | `muxix path <handle>` | The only value taskflow needs. |
| Agent spin-up — default | *(none)* | taskflow `agent` phase with literal `cwd`. The coordinator owns the child. |
| Agent spin-up — opt-in dispatch | `muxix task create` → `muxix task claim` → `muxix add <branch> -b -P <promptfile>` → `muxix wait <handle> --status working --timeout 120` | Claim before create. Prompt files written before any `add`. |
| View another branch | `muxix add <ref> --name <handle> -b -C -F`, `muxix list` | For "what does this file look like on `main`", `git show main:path` beats a whole worktree. |
| Inspect — status | `muxix status`, `muxix list` | `status waiting` means an agent is asking a question. |
| Inspect — output | `muxix capture <handle> -n 50` | Answer a waiting agent with `muxix send <handle> "..."`. |
| Inspect — diff | `muxix run <handle> -- git diff` | There is **no** `muxix diff` subcommand; `run` is the route. |
| Inspect — task store | `muxix task list --json` | `tasks/index.json`, relative to the **main** worktree root, so every worktree shares one graph. |
| Verify | `muxix run <handle> -- <project check command>` | Or run it directly in the taskflow gate phase's `cwd`; same effect, one fewer hop. |
| Cleanup — merge | `muxix merge <handle>` | Merges the branch, then cleans up. Review first. |
| Cleanup — discard | `muxix remove <handle>` | Drops worktree, window and branch. No merge. |
| Cleanup — keep the tree | `muxix close <handle>` | Closes the window only. |
| Recovery | `muxix resurrect --dry-run` | After a tmux or host crash. A **re-prompted** agent lost its history and re-reads its task from scratch — verify its work against the branch rather than assuming it remembers. |

Cross-project: `send`, `capture`, `status`, `wait` and `run` resolve handles across
projects (`project:handle` to disambiguate). Lifecycle commands — `add`, `merge`,
`remove`, `open`, `close` — stay in the current repository.

jj note: a git worktree is **not** colocated for jj. `jj` searches upward and will
read and commit the *parent* repo's working copy while the cwd is the worktree.
Confirm ownership before running jj anywhere:

```sh
[ "$(jj --no-pager workspace root 2>/dev/null)" = "$PWD" ] && echo jj || echo git-only
```

## 5. The taskflow `cwd` step-output gap

Taskflow's `cwd` accepts a literal path, or the exact whole placeholder `{args.X}`
when `X` is declared `type: 'relative-path'` — but **not** `{steps.X.output}`.
So a `script` phase that creates the worktree cannot feed its path to a later
phase's `cwd` inside one flow.

Options considered:

- **(a) Live with two orchestrator calls.** Create the tree and read its path, then
  run the implementation flow with the path inlined as a literal `cwd`.
- **(b) Teach taskflow step-output-as-cwd.** The real fix, but it touches the
  resolve-only host policy and is not a small change.
- **(c) Add `muxix delegate` emitting a predictable path up front**, so the arg
  plumbing works.

**Pick: (a).** Zero new code, works today, and the two-call shape is honest about
where the boundary is. It also keeps the path an explicit, inspectable value rather
than hidden flow state.

(b) is the right thing to do *if* this flow becomes high-traffic enough that the
extra call is real friction — then it collapses to one invocation and benefits every
other workspace-creating flow too. (c) is rejected: it adds a muxix command to work
around a taskflow limitation, which puts the fix in the wrong layer and drags muxix
toward scheduling.
