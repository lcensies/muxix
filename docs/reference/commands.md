# CLI Reference

## Commands overview

| Command | Description |
| ------- | ----------- |
| [`add`](#add) | Create a new worktree and tmux window |
| [`merge`](#merge) | Merge a branch and clean up everything |
| [`remove`](#remove) | Remove worktrees without merging |
| [`list`](#list) | List all worktrees with status |
| [`open`](#open) | Open a tmux window for an existing worktree |
| [`close`](#close) | Close a worktree's tmux window (keeps worktree) |
| [`path`](#path) | Get the filesystem path of a worktree |
| [`init`](#init) | Generate configuration file |
| [`claude prune`](#claude-prune) | Clean up stale Claude Code entries |
| [`completions`](#completions) | Generate shell completions |
| [`docs`](#docs) | Show detailed documentation |

---

## `add`

Creates a new git worktree with a matching tmux window and switches you to it immediately. If the branch doesn't exist, it will be created automatically.

```bash
workmux add <branch-name> [flags]
```

### Arguments

- `<branch-name>`: Name of the branch to create or switch to, a remote branch reference (e.g., `origin/feature-branch`), or a GitHub fork reference (e.g., `user:branch`). Remote and fork references are automatically fetched and create a local branch with the derived name. Optional when using `--pr`.

### Options

| Flag | Description |
| ---- | ----------- |
| `--base <branch\|commit\|tag>` | Specify a base branch, commit, or tag to branch from when creating a new branch. By default, new branches are created from the current branch. |
| `--pr <number>` | Checkout a GitHub pull request by its number. Requires `gh` CLI. |
| `-A, --auto-name` | Generate branch name from prompt using LLM. |
| `--name <name>` | Override the worktree directory and tmux window name. |
| `-b, --background` | Create the tmux window in the background without switching to it. |
| `-w, --with-changes` | Move uncommitted changes from the current worktree to the new worktree. |
| `--patch` | Interactively select which changes to move (requires `--with-changes`). |
| `-u, --include-untracked` | Also move untracked files (requires `--with-changes`). |
| `-p, --prompt <text>` | Provide an inline prompt for AI agent panes. |
| `-P, --prompt-file <path>` | Provide a path to a file for the prompt. |
| `-e, --prompt-editor` | Open your `$EDITOR` to write the prompt interactively. |
| `-a, --agent <name>` | The agent(s) to use. Can be specified multiple times. |
| `-W, --wait` | Block until the created tmux window is closed. |

### Skip options

| Flag | Description |
| ---- | ----------- |
| `-H, --no-hooks` | Skip running `post_create` commands |
| `-F, --no-file-ops` | Skip file copy/symlink operations |
| `-C, --no-pane-cmds` | Skip executing pane commands |

### What happens

1. Determines the **handle** for the worktree by slugifying the branch name (e.g., `feature/auth` becomes `feature-auth`). This can be overridden with `--name`.
2. Creates a git worktree at `<worktree_dir>/<handle>`
3. Runs any configured file operations (copy/symlink)
4. Executes `post_create` commands if defined
5. Creates a new tmux window named `<window_prefix><handle>`
6. Sets up your configured tmux pane layout
7. Automatically switches your tmux client to the new window

### Examples

::: code-group

```bash [Basic usage]
# Create a new branch and worktree
workmux add user-auth

# Use an existing branch
workmux add existing-work

# Create a new branch from a specific base
workmux add hotfix --base production

# Create a worktree from a remote branch
workmux add origin/user-auth-pr

# Create a worktree in the background
workmux add feature/parallel-task --background

# Use a custom name for the worktree
workmux add feature/long-descriptive-branch-name --name short
```

```bash [Pull requests & forks]
# Checkout PR #123
workmux add --pr 123

# Checkout PR #456 with a custom local branch name
workmux add fix/api-bug --pr 456

# Checkout a fork branch using GitHub's owner:branch format
workmux add someuser:feature-branch
```

```bash [Moving changes]
# Move uncommitted changes (including untracked files)
workmux add feature/new-thing --with-changes -u

# Move only staged/modified files
workmux add fix/bug --with-changes

# Interactively select which changes to move
workmux add feature/partial --with-changes --patch
```

```bash [AI agent prompts]
# Create with inline prompt
workmux add feature/ai --prompt "Implement user authentication with OAuth"

# Override the default agent
workmux add feature/testing -a gemini

# Use prompt from a file
workmux add feature/refactor --prompt-file task-description.md

# Open editor to write prompt
workmux add feature/new-api --prompt-editor
```

```bash [Skip setup steps]
# Skip expensive setup for docs-only changes
workmux add docs-update --no-hooks --no-file-ops --no-pane-cmds

# Skip just file operations
workmux add quick-fix --no-file-ops
```

```bash [Scripting with --wait]
# Block until the agent completes
workmux add feature/api --wait -p "Implement the REST API, then run: workmux remove --keep-branch"

# Run sequential agent tasks
for task in task1.md task2.md task3.md; do
  workmux add "task-$(basename $task .md)" --wait -P "$task"
done
```

:::

### Automatic branch name generation

The `--auto-name` (`-A`) flag generates a branch name from your prompt using an LLM via the [`llm`](https://llm.datasette.io/) CLI tool.

```bash
# Opens editor for prompt, generates branch name
workmux add -A

# With inline prompt
workmux add -A -p "Add OAuth authentication"

# With prompt file
workmux add -A -P task-spec.md
```

**Requirements:**

```bash
pipx install llm
llm keys set openai  # or use a local model
```

**Configuration:**

```yaml
auto_name:
  model: 'gemini-2.5-flash-lite'
  system_prompt: |
    Generate a concise git branch name based on the task description.
    Rules:
    - Use kebab-case (lowercase with hyphens)
    - Keep it short: 1-3 words, max 4 if necessary
    - Focus on the core task/feature
    - No prefixes like feat/, fix/, chore/
    Output ONLY the branch name, nothing else.
```

### Parallel workflows & multi-worktree generation

workmux can generate multiple worktrees from a single `add` command.

| Flag | Description |
| ---- | ----------- |
| `-a, --agent <name>` | When used multiple times, creates one worktree for each agent. |
| `-n, --count <number>` | Creates `<number>` worktree instances. |
| `--foreach <matrix>` | Creates worktrees from a variable matrix string. Format: `"var1:valA,valB;var2:valX,valY"` |
| `--branch-template <template>` | MiniJinja template for generating branch names. |
| `--max-concurrent <number>` | Limits how many worktrees run simultaneously. |

**Examples:**

```bash
# Create worktrees for multiple agents
workmux add my-feature -a claude -a gemini -p "Implement search API"
# Generates: my-feature-claude, my-feature-gemini

# Create numbered instances
workmux add my-feature -n 2 -p "Implement task #{{ num }}"
# Generates: my-feature-1, my-feature-2

# Use variable matrix
workmux add my-feature --foreach "platform:iOS,Android" -p "Build for {{ platform }}"
# Generates: my-feature-ios, my-feature-android
```

::: details Prompt file with frontmatter
Instead of passing `--foreach` on the command line, you can specify the variable matrix directly in your prompt file using YAML frontmatter:

```markdown
---
foreach:
  platform: [iOS, Android]
  lang: [swift, kotlin]
---

Build a {{ platform }} app using {{ lang }}.
```

```bash
workmux add mobile-app --prompt-file mobile-task.md
# Generates: mobile-app-ios-swift, mobile-app-android-kotlin
```
:::

::: details Stdin input
Pipe input lines to create multiple worktrees:

```bash
echo -e "api\nauth\ndatabase" | workmux add refactor -P task.md
# {{ input }} = "api", "auth", "database"

# JSON lines parsing
gh repo list --json url,name --jq -c '.[]' | workmux add analyze \
  --branch-template '{{ base_name }}-{{ name }}' \
  -P prompt.md
```
:::

---

## `merge`

Merges a branch into a target branch (main by default) and automatically cleans up all associated resources.

```bash
workmux merge [branch-name] [flags]
```

### Arguments

- `[branch-name]`: Optional name of the branch to merge. If omitted, detects the current branch.

### Options

| Flag | Description |
| ---- | ----------- |
| `--into <branch>` | Merge into the specified branch instead of main. |
| `--ignore-uncommitted` | Commit any staged changes before merging. |
| `--keep, -k` | Keep the worktree, window, and branch after merging. |
| `--rebase` | Rebase the feature branch onto the target before merging. |
| `--squash` | Squash all commits into a single commit. |

### What happens

1. Determines which branch to merge
2. Checks for uncommitted changes
3. Merges your branch into the target using the selected strategy
4. Deletes the tmux window
5. Removes the worktree
6. Deletes the local branch

### Examples

```bash
# Merge branch into main (default: merge commit)
workmux merge user-auth

# Merge the current worktree you're in
workmux merge

# Rebase onto main for linear history
workmux merge user-auth --rebase

# Squash all commits
workmux merge user-auth --squash

# Keep worktree to verify before cleanup
workmux merge user-auth --keep

# Merge into a different branch (stacked PRs)
workmux merge feature/subtask --into feature/parent
```

---

## `remove`

Removes worktrees, tmux windows, and branches without merging. Alias: `rm`

```bash
workmux remove [name]... [flags]
```

### Arguments

- `[name]...`: One or more worktree names. Defaults to current directory.

### Options

| Flag | Description |
| ---- | ----------- |
| `--all` | Remove all worktrees (except main). Prompts for confirmation. |
| `--gone` | Remove worktrees whose upstream remote branch has been deleted. |
| `--force, -f` | Skip confirmation and ignore uncommitted changes. |
| `--keep-branch, -k` | Remove only the worktree and tmux window, keep the branch. |

### Examples

```bash
# Remove the current worktree
workmux remove

# Remove a specific worktree
workmux remove experiment

# Remove multiple worktrees
workmux rm feature-a feature-b feature-c

# Remove worktrees whose remote branches were deleted
workmux rm --gone

# Remove all worktrees
workmux rm --all

# Keep the branch
workmux remove --keep-branch experiment
```

---

## `list`

Lists all git worktrees with their tmux window status and merge status. Alias: `ls`

```bash
workmux list [flags]
```

### Options

| Flag | Description |
| ---- | ----------- |
| `--pr` | Show GitHub PR status for each worktree. Requires `gh` CLI. |

### Example output

```
BRANCH      TMUX    UNMERGED    PATH
------      ----    --------    ----
main        -       -           ~/project
user-auth   ✓       -           ~/project__worktrees/user-auth
bug-fix     ✓       ●           ~/project__worktrees/bug-fix
```

- `✓` in TMUX column = tmux window exists
- `●` in UNMERGED column = branch has commits not merged into main

---

## `open`

Opens or switches to a tmux window for a pre-existing git worktree.

```bash
workmux open <name> [flags]
```

### Options

| Flag | Description |
| ---- | ----------- |
| `-n, --new` | Force opening in a new window even if one exists. |
| `--run-hooks` | Re-runs the `post_create` commands. |
| `--force-files` | Re-applies file copy/symlink operations. |
| `-p, --prompt <text>` | Provide an inline prompt for AI agent panes. |
| `-P, --prompt-file <path>` | Provide a path to a file containing the prompt. |
| `-e, --prompt-editor` | Open your editor to write the prompt. |

### Examples

```bash
# Open or switch to a worktree window
workmux open user-auth

# Force open a second window (creates user-auth-2)
workmux open user-auth --new

# Open with a prompt for AI agents
workmux open user-auth -p "Continue implementing the login flow"

# Re-run hooks and restore files
workmux open user-auth --run-hooks --force-files
```

---

## `close`

Closes the tmux window for a worktree without removing the worktree or branch.

```bash
workmux close [name]
```

### Arguments

- `[name]`: Optional worktree name. Defaults to current directory.

### Examples

```bash
# Close a specific worktree window
workmux close user-auth

# Close the current worktree's window
workmux close
```

::: tip
To reopen the window later, use `workmux open`. You can also use tmux's native kill-window command (`prefix + &`).
:::

---

## `path`

Prints the filesystem path of an existing worktree.

```bash
workmux path <name>
```

### Examples

```bash
# Get the path of a worktree
workmux path user-auth
# Output: /Users/you/project__worktrees/user-auth

# Use in scripts
cd "$(workmux path user-auth)"

# Copy a file to a worktree
cp config.json "$(workmux path feature-branch)/"
```

---

## `init`

Generates `.workmux.yaml` with example configuration and `"<global>"` placeholder usage.

```bash
workmux init
```

---

## `claude prune`

Removes stale entries from Claude config (`~/.claude.json`) that point to deleted worktree directories.

```bash
workmux claude prune
```

### What happens

1. Scans `~/.claude.json` for entries pointing to non-existent directories
2. Creates a backup at `~/.claude.json.bak`
3. Removes all stale entries
4. Reports the number of entries cleaned up

### Example output

```
  - Removing: /Users/user/project__worktrees/old-feature

✓ Created backup at ~/.claude.json.bak
✓ Removed 3 stale entries from ~/.claude.json
```

---

## `completions`

Generates shell completion script for the specified shell.

```bash
workmux completions <shell>
```

### Arguments

- `<shell>`: Shell type: `bash`, `zsh`, or `fish`.

See [Installation - Shell completions](/guide/installation#shell-completions) for setup instructions.

---

## `docs`

Displays the README with terminal formatting.

```bash
workmux docs
```

When run interactively, renders markdown with colors and uses a pager (`less`). When piped (e.g., to an LLM), outputs raw markdown for clean context.
