---
name: muxix
description: Reference for the muxix CLI that manages git worktrees and
  tmux windows as isolated development environments. Use when the user
  mentions muxix, worktrees, or parallel agent workflows.
disable-model-invocation: true
---

# muxix

muxix manages git worktrees paired with tmux windows for parallel
development. Each worktree is an isolated workspace with its own branch,
terminal state, and AI agent.

**If the user asks you to create worktrees or dispatch tasks (e.g.,
"/muxix add ..."), you are a dispatcher.** Write prompt files and run
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

Some commands are not listed in `muxix --help` (its help text is a curated
template). `muxix <cmd> --help` works for every command below.

## Commands

### Create a worktree

```bash
muxix add <branch-name>
```

Creates a git worktree, runs file operations and hooks, creates a tmux
window with configured pane layout, and switches to it.

Key flags:
- `-b, --background`: create without switching to it
- `-p <text>`: inline prompt for AI agent panes
- `-P <file>`: prompt from file
- `-e, --prompt-editor`: write prompt in $EDITOR
- `--prompt-file-only`: write `.muxix/PROMPT-<branch>.md` without injecting
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
muxix list          # all worktrees
muxix list --pr     # with GitHub PR status
muxix list --json   # machine-readable
muxix list <name>   # filter by handle or branch (repeatable)
```

Shows branch, agent status, tmux window status, and unmerged commits.

### Merge a branch

```bash
muxix merge                 # merge current branch into main
muxix merge <branch>        # merge specific branch
muxix merge --rebase        # rebase before merging (linear history)
muxix merge --squash        # squash all commits into one
muxix merge --into <branch> # merge into a different target branch
muxix merge -k, --keep      # merge but keep worktree/window/branch
muxix merge --cleanup       # force cleanup (overrides merge_keep config)
muxix merge -n              # skip pre-merge hooks
muxix merge --no-hooks      # skip pre-merge and pre-remove hooks
muxix merge --ignore-uncommitted
muxix merge --notification  # show system notification on success
```

Merges the branch, deletes the tmux window, removes the worktree, and
deletes the local branch. Use the `/merge` skill for the full workflow
(commit, rebase, then merge).

### Remove worktrees

```bash
muxix remove                # current worktree
muxix remove <name>...      # specific worktrees
muxix rm --gone             # worktrees whose remote branch was deleted
muxix rm --all              # all worktrees
muxix rm -f <name>          # force, skip confirmation
muxix rm -k, --keep-branch  # keep the branch, remove worktree + window
```

### Open / close / rename / restore

```bash
muxix open <name>           # open or switch to tmux window
muxix open -n, --new        # force a new window (creates suffix -2, -3)
muxix open <name> -p "..."  # open with a prompt for agent panes
muxix open <name> -c        # resume the agent's most recent conversation
muxix open <name> --run-hooks --force-files   # re-run setup steps
muxix close <name>          # close tmux window, keep worktree
muxix rename <new>          # rename current worktree + window
muxix rename <old> <new> -b # also rename the git branch
muxix resurrect             # restore windows after a tmux/computer crash
muxix resurrect --dry-run
```

`resurrect` relaunches each crashed worktree's agent. How it comes back
depends on that agent's own session store:

- **Resumable session found** — relaunched with the agent's continue flag,
  picking up the previous conversation.
- **No resumable session** — relaunched fresh, with the stored task prompt
  re-sent (`.muxix/PROMPT-<branch>.md`).
  The re-sent prompt names the parent agent that spawned the worktree, so the
  restored agent knows who to report back to. The original prompt file is
  never overwritten.
- Restored windows are verified alive before being reported, so an agent that
  exits on startup is reported as a failure, not a success.

A common cause of "no resumable session" is changing the configured `agent:`
— the previous agent's sessions are not reachable by the new one.

### Worktree / session journal

`muxix add` records each worktree it creates in the project store
(`.muxix/state/project.json`, one per project, shared by all worktrees):
branch, parent worktree, agent, and the sessions observed in it.

```bash
muxix project-state show    # includes the worktrees journal
```

Sessions are recorded per agent, so a worktree that ran opencode and now runs
claude keeps both and never offers one agent the other's session. The entry is
dropped when the worktree is removed.

The journal is a record of what muxix did, not the source of truth — the
agent's own session store owns the conversation and a user can delete it. Any
recorded session id is verified against that store before it is used.

### Interact with other agents

These commands target agents by their worktree handle. If the handle is
not found in the current repo, muxix searches all active agents globally.
Use `project:handle` syntax to disambiguate when names collide.

```bash
# Check agent statuses
muxix status                          # all agents
muxix status auth api-tests           # specific agents
muxix status --json --git             # machine-readable, with git info

# Wait for agents
muxix wait agent-a agent-b            # block until done
muxix wait agent-a --timeout 3600     # with timeout (seconds)
muxix wait agent-a agent-b --any      # wait for first to finish
muxix wait agent-a --status working   # wait for specific status

# Read agent terminal output
muxix capture agent-a                 # last 200 lines (default)
muxix capture agent-a -n 50           # last 50 lines

# Send instructions to an agent
muxix send agent-a "fix the tests"    # short message
muxix send agent-a "/merge"           # send a skill command
muxix send agent-a -f followup.md     # from file
muxix send myproject:docs "update the API section"  # cross-project

# Run shell commands in an agent's worktree
muxix run agent-a -- pytest tests/    # wait and stream output
muxix run agent-a -b -- npm run build # run in background
muxix run agent-a --timeout 600 -- just test
```

### Navigation and monitoring

```bash
muxix dashboard             # TUI dashboard of all active agents
muxix dashboard -t tasks    # open on a tab: agents|worktrees|tasks|planning|project
muxix dashboard -s          # only agents in the current session
muxix sidebar               # toggle the live agent status sidebar
muxix sidebar -s|-g         # session-scoped / global scope
muxix sidebar next|prev|jump <N>
muxix focus <target>        # switch to an agent by ID or window-name fragment
muxix path <name>           # print worktree filesystem path
```

### Setup and configuration

```bash
muxix init                  # generate .muxix.yaml in current project
muxix setup                 # install status hooks + bundled skills
muxix setup --hooks         # hooks only
muxix setup --skills        # skills only
muxix config edit           # open global config in $EDITOR
muxix config path           # print global config path
muxix config reference      # print default config with all options documented
muxix mcp sync              # render .mcp.json from the `mcp:` config section
muxix mcp status            # show configured MCP servers + integration status
muxix sync-files            # re-apply copy/symlink ops to current worktree
muxix sync-files --all      # ...to all worktrees
muxix sandbox <cmd>         # sandbox image/VM management, sandboxed shells
muxix agent-registry list   # resolved named agent definitions
muxix claude prune          # drop stale ~/.claude.json entries
muxix completions <shell>
muxix docs | changelog | update
```

`muxix setup` also applies the project's `bootstrap:` section: per-agent
plugins, skills, and prompt components (see Configuration below).

### Task graph and orchestration

For unattended multi-task runs. The task graph defaults to
`tasks/index.json`; relative `--graph` paths resolve against the main
worktree root, so agents in a feature worktree share one graph.

```bash
muxix task list                       # all tasks
muxix task list --frontier            # ready tasks (deps satisfied)
muxix task list --status todo --label backend --json
muxix task get <id>                   # exact id, else fuzzy search
muxix task create --title "..." [--id X] [--depends-on Y] [--label L]
                    [--priority N] [--worktree <handle>]
muxix task update <id> ...            # only the flags you pass change
muxix task delete <id>
muxix task list --ready --json        # tasks whose dependencies are all done
muxix task claim <id> --branch <b> --base <b> --worktree <path>
muxix task resolve <id> --retry|--abandon
muxix project-state get-capability|set-fact|get-fact|show
```

muxix stores the graph and serialises writes to it; it never decides what to
run next. Scheduling is the harness's job: read the ready frontier, pick a task,
`muxix add` its worktree, and mark the result back with `muxix task update`.

The `muxix dashboard -t tasks` tab is a view over the same file.

### Org policy and profiles

```bash
muxix provision             # sync org policy and apply it
muxix provision status      # cached org policy
muxix provision sync        # fetch latest from the provision server
muxix provision --dry-run --strict
muxix profile show|export <file>|diff
```

## Configuration

Two levels: global (`~/.config/muxix/config.yaml`) and project
(`.muxix.yaml`). Project overrides global.

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

layouts:                         # named layouts for `muxix add -l <name>`
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

mcp:                             # rendered into .mcp.json by `muxix mcp sync`
  context7:
    command: npx
    args: ["-y", "@upstash/context7-mcp"]

bootstrap:                       # applied by `muxix setup`
  skills:
    - ./skills/muxix           # local path, or {url:, ref:} for a repo
  prompt_components:     # from .muxix/prompt-components/<name>.md
    - fff
  features:                      # agent-agnostic capability -> plugin or prompt
    ponytail:
      pi: git:github.com/DietrichGebert/ponytail
      default: ponytail
  agents:
    claude code:                 # key = agent display name, lowercased
      add_skills:
        - ./skills/worktree
      add_prompt_components:
        - code-review
```

Use `'<global>'` in project config arrays to include global values.

For the full configuration reference with all options documented, run
`muxix config reference`.

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
4. After PR is merged remotely, clean up with `muxix rm --gone`

### Delegating tasks

Use `/worktree` to spin off tasks into parallel worktree agents. The
agent writes a prompt file and runs `muxix add -b -P <file>`.

Spawning, monitoring and merging many of them in a loop is the harness's job:
muxix reports state (`muxix list --json`, `muxix status`, signals) and
performs single actions, it does not run the loop.

### Cross-project worktree creation

`muxix add` creates worktrees in the current git repo and adds the
window to the current tmux session. To create a worktree in a different
project, run `muxix add` inside that project's tmux session.

Discover project paths from existing sessions:

```bash
tmux list-sessions -F '#{session_name} #{session_path}'
```

Then create the worktree in the target session:

```bash
# If the session exists:
tmux new-window -t <session> -c <project-path> \
  "muxix add <branch> -b -P <prompt-file>; exit"

# If the session does not exist, create it first:
tmux new-session -d -s <session> -c <project-path> && \
tmux new-window -t <session> -c <project-path> \
  "muxix add <branch> -b -P <prompt-file>; exit"
```

The temporary window closes when `muxix add` finishes; the worktree
window that muxix creates stays in the session.

Do NOT research before dispatching. Use context you already have, but
do not explore or read code just to write the prompt. Worktree agents
can read files from other projects via absolute paths, so reference
other projects by path and let the agent explore on its own.

## Related Skills

- **`/merge`**: commit, rebase, and merge the current branch
- **`/rebase`**: rebase with smart conflict resolution
- **`/worktree`**: delegate tasks to parallel worktree agents
- **`/open-pr`**: write PR description and open in browser
- **`/implement`**, **`/reject`**: release or refuse a gate the harness is waiting on
