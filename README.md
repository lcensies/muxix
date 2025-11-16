<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="meta/logo-dark.svg">
    <img src="meta/logo.svg" alt="workmux icon" width="300">
  </picture>
</p>

<p align="center">
  <strong>Git worktrees + tmux windows</strong>
</p>

---

Giga opinionated zero-friction workflow tool for managing [git
worktrees](https://git-scm.com/docs/git-worktree) and tmux windows as isolated
development environments.

Perfect for running multiple AI agents in parallel without conflict.

## Philosophy

- **One worktree, one tmux window**: Each git worktree gets its own dedicated,
  pre-configured tmux window
- **Frictionless**: Multi-step workflows reduced to simple commands
- **Configuration as code**: Define your tmux layout and setup steps in
  `.workmux.yaml`

## Features

- Create git worktrees with matching tmux windows in a single command (`add`)
- Automatically set up your preferred pane layout (editor, shell, watchers,
  etc.)
- Run post-creation hooks (install dependencies, setup database, etc.)
- Copy or symlink configuration files (`.env`, `node_modules`) into new
  worktrees
- Start agents with a prompt directly via `--prompt` or `--prompt-file`
- Merge branches and clean up everything (worktree, tmux window, branches) in
  one command (`merge`)
- List all worktrees with their tmux and merge status
- Bootstrap projects with an initial configuration file (`init`)
- Dynamic shell completions for branch names

## Installation

```bash
cargo install workmux
```

## Quick start

1. **Initialize configuration (optional)**:

```bash
workmux init
```

This creates a `.workmux.yaml` file to customize your workflow (pane layouts,
setup commands, file operations, etc.). workmux works out of the box with
sensible defaults, so this step is optional.

2. **Create a new worktree and tmux window**:

```bash
workmux add new-feature
```

This will:

- Create a git worktree at
  `<project_root>/../<project_name>__worktrees/new-feature`
- Create a tmux window named `new-feature`
- Automatically switch your tmux client to the new window

3. **When done, merge and clean up**:

```bash
# Run in the worktree window
workmux merge
```

Merges your branch into main and cleans up everything (tmux window, worktree,
and local branch).

## Configuration

workmux uses a two-level configuration system:

- **Global** (`~/.config/workmux/config.yaml`): Personal defaults for all
  projects
- **Project** (`.workmux.yaml`): Project-specific overrides

Project settings override global settings. For `post_create` and file operation
lists (`files.copy`, `files.symlink`), you can use `"<global>"` to include
global values alongside project-specific ones. Other settings like `panes` are
replaced entirely when defined in the project config.

### Global configuration example

`~/.config/workmux/config.yaml`:

```yaml
window_prefix: wm-

panes:
  - command: nvim .
    focus: true
  # Just a default shell (command omitted)
  - split: horizontal

post_create:
  - mise install

files:
  symlink:
    - node_modules

agent: claude
```

### Project configuration example

`.workmux.yaml`:

```yaml
post_create:
  - '<global>'
  - mise use

files:
  symlink:
    - '<global>' # Include global symlinks (node_modules)
    - .pnpm-store # Add project-specific symlink

panes:
  - command: pnpm install
    focus: true
  - command: <agent>
    split: horizontal
  - command: pnpm run dev
    split: vertical
```

### Configuration options

- `main_branch`: Branch to merge into (optional, auto-detected from remote or
  checks for `main`/`master`)
- `worktree_dir`: Custom directory for worktrees (absolute or relative to repo
  root)
- `window_prefix`: Prefix for tmux window names (default: `wm-`)
- `panes`: Array of pane configurations
  - `command`: Optional command to run when the pane is created. Use this for
    long-running setup like dependency installs so output is visible in tmux.
    If omitted, the pane starts with your default shell. Use `<agent>` to use
    the configured agent.
  - `focus`: Whether this pane should receive focus (default: false)
  - `split`: How to split from previous pane (`horizontal` or `vertical`)
- `post_create`: Commands to run after worktree creation but before the tmux
  window opens. These block window creation, so keep them short (e.g., copying
  config files).
- `files`: File operations to perform on worktree creation
  - `copy`: List of glob patterns for files/directories to copy
  - `symlink`: List of glob patterns for files/directories to symlink
- `agent`: The default agent command to use for `<agent>` in pane commands
  (e.g., `claude`, `gemini`). This can be overridden by the `--agent` flag.
  Default: `claude`.

#### Default behavior

- Worktrees are created in `<project>__worktrees` as a sibling directory to your
  project by default
- If no `panes` configuration is defined, workmux provides opinionated defaults:
  - For projects with a `CLAUDE.md` file: Opens the configured agent (see
    `agent` option) in the first pane, defaulting to `claude` if none is set.
  - For all other projects: Opens your default shell.
  - Both configurations include a second pane split horizontally
- `post_create` commands are optional and only run if you configure them

### Directory structure

Here's how workmux organizes your worktrees by default:

```
~/projects/
├── my-project/               <-- Main project directory
│   ├── src/
│   ├── package.json
│   └── .workmux.yaml
│
└── my-project__worktrees/    <-- Worktrees created by workmux
    ├── feature-A/            <-- Isolated workspace for 'feature-A' branch
    │   ├── src/
    │   └── package.json
    │
    └── bugfix-B/             <-- Isolated workspace for 'bugfix-B' branch
        ├── src/
        └── package.json
```

Each worktree is a separate working directory for a different branch, all sharing
the same git repository. This allows you to work on multiple branches
simultaneously without conflicts.

You can customize the worktree directory location using the `worktree_dir`
configuration option (see [Configuration options](#configuration-options)).

### Shell alias (recommended)

For faster typing, alias `workmux` to `wm`:

```bash
alias wm='workmux'
```

## Commands

- [`add`](#workmux-add-branch-name) - Create a new worktree and tmux window
- [`merge`](#workmux-merge-branch-name) - Merge a branch and clean up everything
- [`remove`](#workmux-remove-branch-name) - Remove a worktree without merging
- [`list`](#workmux-list) - List all worktrees with status
- [`init`](#workmux-init) - Generate configuration file
- [`open`](#workmux-open-branch-name) - Open a tmux window for an existing
  worktree
- [`claude prune`](#workmux-claude-prune) - Clean up stale Claude Code entries
- [`completions`](#workmux-completions-shell) - Generate shell completions

### `workmux add <branch-name>`

Creates a new git worktree with a matching tmux window and switches you to it
immediately. If the branch doesn't exist, it will be created automatically.

- `<branch-name>`: Name of the branch to create or switch to, or a remote branch
  reference (e.g., `origin/feature-branch`). When you provide a remote reference,
  workmux automatically fetches it and creates a local branch with the name derived
  from the remote branch (e.g., `origin/feature/foo` creates local branch `feature/foo`).

#### Useful options

- `--base <branch|commit|tag>`: Specify a base branch, commit, or tag to branch from
  when creating a new branch. By default, new branches are created from your main
  branch's remote tracking branch (e.g., `origin/main`).
- `-c, --from-current`: Use your currently checked out branch as the base. This is a
  shorthand for passing that branch explicitly via `--base` and is helpful when
  stacking feature branches.
- `-p, --prompt <text>`: Provide an inline prompt that will be automatically passed to
  AI agent panes.
- `-P, --prompt-file <path>`: Provide a path to a file whose contents will be used as the
  prompt.
- `-e, --prompt-editor`: Open your `$EDITOR` (or `$VISUAL`) to write the prompt interactively.
- `-a, --agent <name>`: Override the default agent for this worktree (e.g., `gemini`).

Note: The prompt options are mutually exclusive - you can only use one at a time.

#### Skip options

These options allow you to skip expensive setup steps when they're not needed (e.g., for
documentation-only changes):

- `-H, --no-hooks`: Skip running `post_create` commands
- `-F, --no-file-ops`: Skip file copy/symlink operations (e.g., skip linking `node_modules`)
- `-C, --no-pane-cmds`: Skip executing pane commands (panes open with plain shells instead)

#### What happens

1. Creates a git worktree at
   `<project_root>/../<project_name>__worktrees/<branch-name>`
2. Runs any configured file operations (copy/symlink)
3. Executes `post_create` commands if defined (runs before the tmux window opens, so keep
   them fast)
4. Creates a new tmux window named after the branch
5. Sets up your configured tmux pane layout
6. Automatically switches your tmux client to the new window

#### Examples

```bash
# Create a new branch and worktree
workmux add user-auth

# Use an existing branch
workmux add existing-work

# Create a new branch from a specific base
workmux add hotfix --base production

# Use the current branch as the base (stacked branch)
workmux add feature-2 --from-current

# Create a worktree from a remote branch (creates local branch "user-auth-pr")
workmux add origin/user-auth-pr

# Remote branches with slashes work too (creates local branch "feature/foo")
workmux add origin/feature/foo

# Create a worktree with an inline prompt for AI agents
workmux add feature/ai --prompt "Implement user authentication with OAuth"

# Override the default agent for a specific worktree
workmux add feature/testing -a gemini

# Create a worktree with a prompt from a file
workmux add feature/refactor --prompt-file task-description.md

# Open your editor to write a prompt interactively
workmux add feature/new-api --prompt-editor

# Skip expensive setup for documentation-only changes
workmux add docs-update --no-hooks --no-file-ops --no-pane-cmds

# Skip just the file operations (e.g., you don't need node_modules)
workmux add quick-fix --no-file-ops
```

#### AI agent integration

When you provide a prompt via `--prompt`, `--prompt-file`, or
`--prompt-editor`, workmux automatically injects the prompt into panes running
the configured agent command (e.g., `claude`, `gemini`, or whatever you've set
via the `agent` config or `--agent` flag) without requiring any `.workmux.yaml`
changes:

- Panes with a command matching the configured agent are automatically started
  with the given prompt.
- You can keep your `.workmux.yaml` pane configuration simple (e.g., `panes: [{
command: "<agent>" }]`) and let workmux handle prompt injection at runtime.

This means you can launch AI agents with task-specific prompts without modifying your
project configuration for each task.

---

### `workmux merge [branch-name]`

Merges a branch into the main branch and automatically cleans up all associated
resources (worktree, tmux window, and local branch).

- `[branch-name]`: Optional name of the branch to merge. If omitted,
  automatically detects the current branch from the worktree you're in.

#### Useful options

- `--ignore-uncommitted`: Commit any staged changes before merging without
  opening an editor
- `--delete-remote`, `-r`: Also delete the remote branch after a successful
  merge

#### Merge strategies

By default, `workmux merge` performs a standard merge commit. You can customize
the merge behavior with these mutually exclusive flags:

- `--rebase`: Rebase the feature branch onto main before merging (creates a
  linear history via fast-forward merge). If conflicts occur, you'll need to
  resolve them manually in the worktree and run `git rebase --continue`.
- `--squash`: Squash all commits from the feature branch into a single commit on
  main. You'll be prompted to provide a commit message in your editor.

#### What happens

1. Determines which branch to merge (specified branch or current branch if
   omitted)
2. Checks for uncommitted changes (errors if found, unless
   `--ignore-uncommitted` is used)
3. Commits staged changes if present (unless `--ignore-uncommitted` is used)
4. Merges your branch into main using the selected strategy (default: merge
   commit)
5. Deletes the tmux window (including the one you're currently in if you ran
   this from a worktree)
6. Removes the worktree
7. Deletes the local branch

#### Typical workflow

When you're done working in a worktree, simply run `workmux merge` from within
that worktree's tmux window. The command will automatically detect which branch
you're on, merge it into main, and close the current window as part of cleanup.

#### Examples

```bash
# Merge branch from main branch (default: merge commit)
workmux merge user-auth

# Merge the current worktree you're in
# (run this from within the worktree's tmux window)
workmux merge

# Rebase onto main before merging for a linear history
workmux merge user-auth --rebase

# Squash all commits into a single commit
workmux merge user-auth --squash

# Merge and also delete the remote branch
workmux merge user-auth --delete-remote
```

---

### `workmux remove <branch-name>` (alias: `rm`)

Removes a worktree, tmux window, and branch without merging (unless you keep
the branch). Useful for abandoning work or cleaning up experimental branches.

- `<branch-name>`: Name of the branch to remove.

#### Useful options

- `--force`, `-f`: Skip confirmation prompt and ignore uncommitted changes
- `--delete-remote`, `-r`: Also delete the remote branch
- `--keep-branch`, `-k`: Remove only the worktree and tmux window while keeping
  the local branch (incompatible with `--delete-remote`)

#### Examples

```bash
# Remove with confirmation if unmerged
workmux remove experiment

# Use the alias
workmux rm old-work

# Remove worktree/window but keep the branch
workmux remove --keep-branch experiment

# Force remove without prompts
workmux rm -f experiment

# Force remove and delete remote branch
workmux rm -f -r old-work
```

---

### `workmux list` (alias: `ls`)

Lists all git worktrees with their tmux window status and merge status.

#### Examples

```bash
# List all worktrees
workmux list
```

#### Example output

```
BRANCH      TMUX    UNMERGED    PATH
------      ----    --------    ----
main        -       -           ~/project
user-auth   ✓       -           ~/project__worktrees/user-auth
bug-fix     ✓       ●           ~/project__worktrees/bug-fix
```

#### Key

- `✓` in TMUX column = tmux window exists for this worktree
- `●` in UNMERGED column = branch has commits not merged into main
- `-` = not applicable

---

### `workmux init`

Generates `.workmux.yaml` with example configuration and `"<global>"`
placeholder usage.

#### Examples

```bash
workmux init
```

---

### `workmux open <branch-name>`

Opens a new tmux window for a pre-existing git worktree, setting up the
configured pane layout and environment. This is useful any time you closed the
tmux window for a worktree you are still working on.

- `<branch-name>`: Name of the branch that has an existing worktree.

#### Useful options

- `--run-hooks`: Re-runs the `post_create` commands (these block window creation).
- `--force-files`: Re-applies file copy/symlink operations. Useful for restoring
  a deleted `.env` file.

#### What happens

1. Verifies that a worktree for `<branch-name>` exists and a tmux window does
   not.
2. Creates a new tmux window named after the branch.
3. (If specified) Runs file operations and `post_create` hooks.
4. Sets up your configured tmux pane layout.
5. Automatically switches your tmux client to the new window.

#### Examples

```bash
# Open a window for an existing worktree
workmux open user-auth

# Open and re-run dependency installation
workmux open user-auth --run-hooks

# Open and restore configuration files
workmux open user-auth --force-files
```

---

### `workmux claude prune`

Removes stale entries from Claude config (`~/.claude.json`) that point to
deleted worktree directories. When you run Claude Code in worktrees, it stores
per-worktree settings in that file. Over time, as worktrees are merged or
deleted, it can accumulate entries for paths that no longer exist.

#### What happens

1. Scans `~/.claude.json` for entries pointing to non-existent directories
2. Creates a backup at `~/.claude.json.bak` before making changes
3. Removes all stale entries
4. Reports the number of entries cleaned up

#### Safety

- Only removes entries for absolute paths that don't exist
- Creates a backup before modifying the file
- Preserves all valid entries and relative paths

#### Examples

```bash
# Clean up stale Claude Code entries
workmux claude prune
```

#### Example output

```
  - Removing: /Users/user/project__worktrees/old-feature

✓ Created backup at ~/.claude.json.bak
✓ Removed 3 stale entries from ~/.claude.json
```

---

### `workmux completions <shell>`

Generates shell completion script for the specified shell. Completions provide
tab-completion for commands and dynamic branch name suggestions.

- `<shell>`: Shell type: `bash`, `zsh`, or `fish`.

#### Examples

```bash
# Generate completions for zsh
workmux completions zsh
```

See the [Shell Completions](#shell-completions) section for installation
instructions.

## Workflow example

Here's a complete workflow:

```bash
# Start a new feature
workmux add user-auth

# Work on your feature...
# (tmux automatically sets up your configured panes and environment)

# When ready, merge and clean up
workmux merge user-auth

# Start another feature
workmux add api-endpoint

# List all active worktrees
workmux list
```

## Why workmux?

`workmux` reduces manual setup to a pair of commands and makes it feasible to
run AI-driven development workflows in parallel.

### Without workmux

```bash
# 1. Manually create the worktree and environment
git worktree add ../worktrees/user-auth -b user-auth
cd ../worktrees/user-auth
cp ../../project/.env.example .env
ln -s ../../project/node_modules .
npm install
# ... and other setup steps

# 2. Manually create and configure the tmux window
tmux new-window -n user-auth
tmux split-window -h 'npm run dev'
tmux send-keys -t 0 'claude' C-m
# ... repeat for every pane in your desired layout

# 3. When done, manually merge and clean everything up
cd ../../project
git switch main && git pull
git merge --no-ff user-auth
tmux kill-window -t user-auth
git worktree remove ../worktrees/user-auth
git branch -d user-auth
```

### With workmux

```bash
# Create the environment
workmux add user-auth

# ... work on the feature ...

# Merge and clean up
workmux merge
```

### The parallel AI workflow (with workmux)

Delegate multiple complex tasks to AI agents and let them work at the same time.
This workflow is cumbersome to manage manually.

```bash
# Task 1: Refactor the user model (for Agent 1)
workmux add refactor/user-model

# Task 2: Build a new API endpoint (for Agent 2, in parallel)
workmux add feature/new-api

# ... Command agents work simultaneously in their isolated environments ...

# Merge each task as it's completed
workmux merge refactor/user-model
workmux merge feature/new-api
```

## Tips

### Closing tmux windows

You can close workmux-managed tmux windows using tmux's standard `kill-window`
command (e.g., `<prefix> &` or `tmux kill-window -t <window-name>`). This will
properly terminate all processes running in the window's panes. The git worktree
will remain on disk, and you can reopen a window for it anytime with:

```bash
workmux open <branch-name>
```

However, it's recommended to use `workmux merge` or `workmux remove` for
cleanup instead, as these commands clean up both the tmux window and the git
worktree together. Use `workmux list` to see which worktrees have detached tmux
windows.

## Shell completions

To enable tab completions for commands and branch names, add the following to
your shell's configuration file.

For **bash**, add to your `.bashrc`:

```bash
eval "$(workmux completions bash)"
```

For **zsh**, add to your `.zshrc`:

```bash
eval "$(workmux completions zsh)"
```

For **fish**, add to your `config.fish`:

```bash
workmux completions fish | source
```

## Requirements

- Rust (for building)
- Git 2.5+ (for worktree support)
- tmux

## Inspiration and related tools

workmux is inspired by [wtp](https://github.com/satococoa/wtp), an excellent git
worktree management tool. While wtp streamlines worktree creation and setup,
workmux takes this further by tightly coupling worktrees with tmux window
management.

For managing multiple AI agents in parallel, tools like
[claude-squad](https://github.com/smtg-ai/claude-squad) and
[vibe-kanban](https://github.com/BloopAI/vibe-kanban/) offer dedicated
interfaces, like a TUI or kanban board. workmux takes a different approach:
**tmux is the interface**. If you already live in tmux, you don't need a new app
or abstraction layer. With workmux, managing parallel agents is managing tmux
windows.

## See also

- [tmux-bro](https://github.com/raine/tmux-bro)
- [tmux-file-picker](https://github.com/raine/tmux-file-picker)
