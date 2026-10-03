---
name: workmux
description: Reference for the workmux CLI that manages git worktrees and
  tmux windows as isolated development environments. Use when the user
  mentions workmux, worktrees, or parallel agent workflows.
disable-model-invocation: true
---

# workmux

workmux manages git worktrees paired with tmux windows for parallel
development. Each worktree is an isolated workspace with its own branch,
terminal state, and AI agent.

**If the user asks you to create worktrees or dispatch tasks (e.g.,
"/workmux add ..."), you are a dispatcher.** Write prompt files and run
commands. Do NOT explore, read, or research the codebase first. Use
context you already have. The worktree agent does all the work.

## Key Concepts

- **Handle**: the worktree directory name, derived from the branch name
  (slugified per `worktree_naming`). Used to identify worktrees in all
  commands
- **Worktree directory**: defaults to `<project>__worktrees/<handle>` as a
  sibling of the project root (`worktree_dir` overrides)
- **Window prefix**: tmux windows are named `wm-<handle>` by default
  (configurable via `window_prefix`; `worktree_prefix` prefixes both the
  directory and the window name)
- **Agent status**: agents report status via hooks: working, waiting (needs
  input), done (finished)
- **Mode**: `window` (default) puts each worktree in a window of the current
  tmux session; `session` gives each worktree its own session

Some commands are not listed in `workmux --help` (its help text is a curated
template). `workmux <cmd> --help` works for every command below.

## Commands

### Create a worktree

```bash
workmux add <branch-name>
```

Creates a git worktree, runs file operations and hooks, creates a tmux
window with configured pane layout, and switches to it.

Key flags:
- `-b, --background`: create without switching to it
- `-p <text>`: inline prompt for AI agent panes
- `-P <file>`: prompt from file
- `-e, --prompt-editor`: write prompt in $EDITOR
- `--prompt-file-only`: write `.workmux/PROMPT-<branch>.md` without injecting
  it into an agent pane
- `-A, --auto-name`: generate branch name from prompt via LLM
- `-a <agent>`: override the agent (repeatable — one worktree per agent)
- `-w, --with-changes`: move uncommitted changes to the new worktree
  (`--patch` to select interactively, `-u` to include untracked)
- `--base <branch>`: branch from a specific base
- `--pr <number>`: check out a GitHub PR (positional arg becomes the local
  branch name)
- `--name <name>`: override the handle name
- `-o, --open-if-exists`: open existing worktree if it exists (idempotent)
- `-W, --wait`: block until the tmux window is closed
- `-n, --count <N>`: create N worktree instances
- `--foreach <matrix>`: create worktrees from a variable matrix
  (`"var1:a,b;var2:x,y"`)
- `--branch-template <tpl>`: branch naming for multi-worktree modes
- `--max-concurrent <N>`: cap concurrent worktrees in multi-worktree modes
- `-l, --layout <name>`: use a named pane layout from `layouts:` config
- `--fork[=<session-id>]`: fork the current worktree's conversation into the
  new one
- `-s, --session` / `--mode <window|session>`: multiplexer mode override
- `-S, --sandbox`: force sandbox mode on for this run
- `--config <file>`: alternate config file for this invocation
- `-H, --no-hooks`, `-F, --no-file-ops`, `-C, --no-pane-cmds`: skip setup steps

### List worktrees

```bash
workmux list          # all worktrees
workmux list --pr     # with GitHub PR status
workmux list --json   # machine-readable
workmux list <name>   # filter by handle or branch (repeatable)
```

Shows branch, agent status, tmux window status, and unmerged commits.

### Merge a branch

```bash
workmux merge                 # merge current branch into main
workmux merge <branch>        # merge specific branch
workmux merge --rebase        # rebase before merging (linear history)
workmux merge --squash        # squash all commits into one
workmux merge --into <branch> # merge into a different target branch
workmux merge -k, --keep      # merge but keep worktree/window/branch
workmux merge --cleanup       # force cleanup (overrides merge_keep config)
workmux merge -n              # skip pre-merge hooks
workmux merge --no-hooks      # skip pre-merge and pre-remove hooks
workmux merge --ignore-uncommitted
workmux merge --notification  # show system notification on success
```

Merges the branch, deletes the tmux window, removes the worktree, and
deletes the local branch. Use the `/merge` skill for the full workflow
(commit, rebase, then merge).

### Remove worktrees

```bash
workmux remove                # current worktree
workmux remove <name>...      # specific worktrees
workmux rm --gone             # worktrees whose remote branch was deleted
workmux rm --all              # all worktrees
workmux rm -f <name>          # force, skip confirmation
workmux rm -k, --keep-branch  # keep the branch, remove worktree + window
```

### Open / close / rename / restore

```bash
workmux open <name>           # open or switch to tmux window
workmux open -n, --new        # force a new window (creates suffix -2, -3)
workmux open <name> -p "..."  # open with a prompt for agent panes
workmux open <name> -c        # resume the agent's most recent conversation
workmux open <name> --run-hooks --force-files   # re-run setup steps
workmux close <name>          # close tmux window, keep worktree
workmux rename <new>          # rename current worktree + window
workmux rename <old> <new> -b # also rename the git branch
workmux resurrect             # restore windows after a tmux/computer crash
workmux resurrect --dry-run
```

`resurrect` relaunches each crashed worktree's agent. How it comes back
depends on that agent's own session store:

- **Resumable session found** — relaunched with the agent's continue flag,
  picking up the previous conversation.
- **No resumable session** — relaunched fresh, with the stored task prompt
  re-sent (`.workmux/PROMPT-<branch>.md`, or the orchestrate task workflow).
  The re-sent prompt names the parent agent that spawned the worktree, so the
  restored agent knows who to report back to. The original prompt file is
  never overwritten.
- Restored windows are verified alive before being reported, so an agent that
  exits on startup is reported as a failure, not a success.

A common cause of "no resumable session" is changing the configured `agent:`
— the previous agent's sessions are not reachable by the new one.

### Worktree / session journal

`workmux add` records each worktree it creates in the project store
(`.workmux/state/project.json`, one per project, shared by all worktrees):
branch, parent worktree, agent, and the sessions observed in it.

```bash
workmux project-state show    # includes the worktrees journal
```

Sessions are recorded per agent, so a worktree that ran opencode and now runs
claude keeps both and never offers one agent the other's session. The entry is
dropped when the worktree is removed.

The journal is a record of what workmux did, not the source of truth — the
agent's own session store owns the conversation and a user can delete it. Any
recorded session id is verified against that store before it is used.

### Interact with other agents

These commands target agents by their worktree handle. If the handle is
not found in the current repo, workmux searches all active agents globally.
Use `project:handle` syntax to disambiguate when names collide.

```bash
# Check agent statuses
workmux status                          # all agents
workmux status auth api-tests           # specific agents
workmux status --json --git             # machine-readable, with git info

# Wait for agents
workmux wait agent-a agent-b            # block until done
workmux wait agent-a --timeout 3600     # with timeout (seconds)
workmux wait agent-a agent-b --any      # wait for first to finish
workmux wait agent-a --status working   # wait for specific status

# Read agent terminal output
workmux capture agent-a                 # last 200 lines (default)
workmux capture agent-a -n 50           # last 50 lines

# Send instructions to an agent
workmux send agent-a "fix the tests"    # short message
workmux send agent-a "/merge"           # send a skill command
workmux send agent-a -f followup.md     # from file
workmux send myproject:docs "update the API section"  # cross-project

# Run shell commands in an agent's worktree
workmux run agent-a -- pytest tests/    # wait and stream output
workmux run agent-a -b -- npm run build # run in background
workmux run agent-a --timeout 600 -- just test
```

### Navigation and monitoring

```bash
workmux dashboard             # TUI dashboard of all active agents
workmux dashboard -t tasks    # open on a tab: agents|worktrees|tasks|planning|project
workmux dashboard -s          # only agents in the current session
workmux sidebar               # toggle the live agent status sidebar
workmux sidebar -s|-g         # session-scoped / global scope
workmux sidebar next|prev|jump <N>
workmux focus <target>        # switch to an agent by ID or window-name fragment
workmux path <name>           # print worktree filesystem path
```

### Setup and configuration

```bash
workmux init                  # generate .workmux.yaml in current project
workmux setup                 # install status hooks + bundled skills
workmux setup --hooks         # hooks only
workmux setup --skills        # skills only
workmux config edit           # open global config in $EDITOR
workmux config path           # print global config path
workmux config reference      # print default config with all options documented
workmux mcp sync              # render .mcp.json from the `mcp:` config section
workmux mcp status            # show configured MCP servers + integration status
workmux sync-files            # re-apply copy/symlink ops to current worktree
workmux sync-files --all      # ...to all worktrees
workmux sandbox <cmd>         # sandbox image/VM management, sandboxed shells
workmux agent-registry list   # resolved named agent definitions
workmux claude prune          # drop stale ~/.claude.json entries
workmux completions <shell>
workmux docs | changelog | update
```

`workmux setup` also applies the project's `bootstrap:` section: per-agent
plugins, skills, and prompt components (see Configuration below).

### Task graph and orchestration

For unattended multi-task runs. The task graph defaults to
`tasks/index.json`; relative `--graph` paths resolve against the main
worktree root, so agents in a feature worktree share one graph.

```bash
workmux task list                       # all tasks
workmux task list --frontier            # ready tasks (deps satisfied)
workmux task list --status todo --label backend --json
workmux task get <id>                   # exact id, else fuzzy search
workmux task create --title "..." [--id X] [--depends-on Y] [--label L]
                    [--priority N] [--worktree <handle>]
workmux task update <id> ...            # only the flags you pass change
workmux task delete <id>
workmux graph [--all]                   # frontier / stats
workmux tasks                           # interactive task graph TUI

workmux orchestrate                     # run the loop over the graph
    [--slots N] [--workflow <yaml>] [--auto-merge] [--base-branch <b>]
    [--pre-merge-cmd "..."] [--dry-run]
workmux notify --task-id <id> [--status done]   # signal completion

workmux pipeline run <workflow.yaml>    # single DAG run
workmux pipeline validate <workflow.yaml>
workmux pipeline tui | orchestrator-tui
workmux pipeline approve <node> | reject <node> --feedback "..."

workmux daemon start|stop|restart|status
workmux project-state get-capability|set-fact|get-fact|show
```

Inside a pipeline, `/implement` and `/approve` release approval gates from
the agent pane.

### Org policy and profiles

```bash
workmux provision             # sync org policy and apply it
workmux provision status      # cached org policy
workmux provision sync        # fetch latest from the provision server
workmux provision --dry-run --strict
workmux profile show|export <file>|diff
```

## Configuration

Two levels: global (`~/.config/workmux/config.yaml`) and project
(`.workmux.yaml`). Project overrides global.

### Key options

```yaml
agent: claude                    # default agent for <agent> placeholder
main_branch: main                # merge target (auto-detected by default)
base_branch: develop             # default base for new worktrees
merge_strategy: rebase           # merge, rebase, or squash
merge_keep: false                # keep worktree/branch after merge
mode: window                     # window or session
worktree_dir: .worktrees         # supports ~ and {project}
worktree_naming: full            # full or basename
worktree_prefix: ""              # prefixes worktree dir and window name
window_prefix: wm-               # tmux window name prefix

panes:
  - command: <agent>             # <agent> resolves to configured agent
    focus: true
  - split: horizontal            # second pane with shell

layouts:                         # named layouts for `workmux add -l <name>`
  review:
    panes:
      - command: <agent>

files:
  copy:
    - .env                       # copy from main worktree
  symlink:
    - node_modules               # symlink from main worktree

post_create:
  - '<global>'                   # include global hooks
  - npm install
pre_merge:
  - just test
pre_remove: []

mcp:                             # rendered into .mcp.json by `workmux mcp sync`
  context7:
    command: npx
    args: ["-y", "@upstash/context7-mcp"]

bootstrap:                       # applied by `workmux setup`
  default_skills:
    - ./skills/workmux           # local path, or {url:, ref:} for a repo
  default_prompt_components:     # from .workmux/prompt-components/<name>.md
    - fff
  features:                      # agent-agnostic capability -> plugin or prompt
    ponytail:
      pi: git:github.com/DietrichGebert/ponytail
      default: ponytail
  agents:
    claude code:                 # key = agent display name, lowercased
      additional_skills:
        - ./skills/worktree
      additional_prompt_components:
        - code-review
```

Use `'<global>'` in project config arrays to include global values.

For the full configuration reference with all options documented, run
`workmux config reference`.

### Agent detection

Built-in agents (`claude`, `gemini`, `codex`, `opencode`, `copilot`, `pi`,
`omp`) are auto-detected in pane commands and receive prompt injection
automatically. The `<agent>` placeholder resolves to the configured agent.

Skills are installed per agent into that agent's own skills directory
(`~/.claude/skills`, `~/.config/opencode/skills`, `~/.pi/agent/skills`,
`~/.omp/agent/skills`). Codex, Copilot, and Gemini have no skills directory
and are skipped.

## Common Workflows

### Finishing work: direct merge

Use `/merge` to commit, rebase onto the base branch, and merge in one
step. This cleans up the worktree, tmux window, and branch.

### Finishing work: PR-based

1. Commit changes
2. `git push -u origin HEAD`
3. Use `/open-pr` to write a PR description and open in browser
4. After PR is merged remotely, clean up with `workmux rm --gone`

### Delegating tasks

Use `/worktree` to spin off tasks into parallel worktree agents. The
agent writes a prompt file and runs `workmux add -b -P <file>`.

For full lifecycle orchestration (spawn, monitor, merge), use
`/coordinator`.

### Cross-project worktree creation

`workmux add` creates worktrees in the current git repo and adds the
window to the current tmux session. To create a worktree in a different
project, run `workmux add` inside that project's tmux session.

Discover project paths from existing sessions:

```bash
tmux list-sessions -F '#{session_name} #{session_path}'
```

Then create the worktree in the target session:

```bash
# If the session exists:
tmux new-window -t <session> -c <project-path> \
  "workmux add <branch> -b -P <prompt-file>; exit"

# If the session does not exist, create it first:
tmux new-session -d -s <session> -c <project-path> && \
tmux new-window -t <session> -c <project-path> \
  "workmux add <branch> -b -P <prompt-file>; exit"
```

The temporary window closes when `workmux add` finishes; the worktree
window that workmux creates stays in the session.

Do NOT research before dispatching. Use context you already have, but
do not explore or read code just to write the prompt. Worktree agents
can read files from other projects via absolute paths, so reference
other projects by path and let the agent explore on its own.

## Related Skills

- **`/merge`**: commit, rebase, and merge the current branch
- **`/rebase`**: rebase with smart conflict resolution
- **`/worktree`**: delegate tasks to parallel worktree agents
- **`/coordinator`**: orchestrate multiple agents (spawn, monitor, merge)
- **`/open-pr`**: write PR description and open in browser
- **`/implement`**, **`/approve`**: release pipeline approval gates
