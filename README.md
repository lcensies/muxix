<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="meta/logo-dark.svg">
    <img src="meta/logo.svg" alt="workmux icon" width="300">
  </picture>
</p>

<p align="center">
  <strong>Parallel development in tmux with git worktrees</strong>
</p>

<p align="center">
  <a href="https://workmux.raine.dev/"><strong>📖 Documentation</strong></a> ·
  <a href="#why-git-worktrees">Why?</a> ·
  <a href="#installation">Install</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#commands">Commands</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

---

Giga opinionated zero-friction workflow tool for managing
[git worktrees](https://git-scm.com/docs/git-worktree) and tmux windows as
isolated development environments. Perfect for running multiple AI agents in
parallel without conflict.

📖 **New to workmux?** Read the
[introduction blog post](https://raine.dev/blog/introduction-to-workmux/) for a
quick overview.

🚀 **Using Claude Code?** Try the
[`/worktree` command](#delegating-tasks-with-a-custom-command) to delegate tasks
from your conversation.

![workmux demo](https://raw.githubusercontent.com/raine/workmux/refs/heads/main/meta/demo.gif)

## Philosophy

- **Native tmux integration**: Workmux creates windows in your current tmux
  session. Your existing shortcuts, themes, and workflow stay intact.
- **One worktree, one tmux window**: Each git worktree gets its own dedicated,
  pre-configured tmux window.
- **Frictionless**: Multi-step workflows are reduced to simple commands.
- **Configuration as code**: Define your tmux layout and setup steps in
  `.workmux.yaml`.

The core principle is that **tmux is the interface**. If you already live in
tmux, you shouldn't need to learn a new TUI app or separate interface to manage
your work. With workmux, managing parallel development tasks, or multiple AI
agents, is as simple as managing tmux windows.

## Features

- Create git worktrees with matching tmux windows in a single command (`add`)
- Merge branches and clean up everything (worktree, tmux window, branches) in
  one command (`merge`)
- Automatically set up your preferred tmux pane layout (editor, shell, watchers,
  etc.)
- Run post-creation hooks (install dependencies, setup database, etc.)
- Copy or symlink configuration files (`.env`, `node_modules`) into new
  worktrees
- [Delegate tasks to worktree agents](#delegating-tasks-with-a-custom-command)
  with a `/worktree` slash command
- [Automatic branch name generation](#automatic-branch-name-generation) from
  prompts using LLM
- [Display Claude agent status in tmux window names](#agent-status-tracking)
- Shell completions

## Hype

> "I've been using (and loving) workmux which brings together tmux, git
> worktrees, and CLI agents into an opinionated workflow."  
> — @Coolin96 [🔗](https://news.ycombinator.com/item?id=46029809)

> "Thank you so much for your work with workmux! It's a tool I've been wanting
> to exist for a long time."  
> — @rstacruz [🔗](https://github.com/raine/workmux/issues/2)

## Installation

### Homebrew (macOS/Linux)

```bash
brew install raine/workmux/workmux
```

### Pre-built binaries

Download the [latest release](https://github.com/raine/workmux/releases/latest)
for your platform:

| Platform              | Download                                                                                                             |
| --------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Linux (x64)           | [workmux-linux-amd64.tar.gz](https://github.com/raine/workmux/releases/latest/download/workmux-linux-amd64.tar.gz)   |
| Linux (ARM64)         | [workmux-linux-arm64.tar.gz](https://github.com/raine/workmux/releases/latest/download/workmux-linux-arm64.tar.gz)   |
| macOS (Intel)         | [workmux-darwin-amd64.tar.gz](https://github.com/raine/workmux/releases/latest/download/workmux-darwin-amd64.tar.gz) |
| macOS (Apple Silicon) | [workmux-darwin-arm64.tar.gz](https://github.com/raine/workmux/releases/latest/download/workmux-darwin-arm64.tar.gz) |

Extract and install:

```bash
tar xzf workmux-*.tar.gz
sudo mv workmux /usr/local/bin/
```

### Cargo

Requires Rust. Install via [rustup](https://rustup.rs/) if you don't have it.

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
   - Create a tmux window named `wm-new-feature` (the prefix is configurable)
   - Automatically switch your tmux client to the new window

3. **Do your thing**

4. **When done, merge and clean up**:

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
window_prefix: "\uf418 " # Use nerdtree branch icon as prefix
merge_strategy: rebase # Make workmux merge do rebase by default
agent: claude

panes:
  - command: <agent> # Start the configured agent (e.g., claude)
    focus: true
  - split: horizontal # Second pane with default shell
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

For a real-world example, see
[workmux's own `.workmux.yaml`](https://github.com/raine/workmux/blob/main/.workmux.yaml).

### Configuration options

Most options have sensible defaults. You only need to configure what you want to
customize.

#### Basic options

| Option           | Description                                          | Default                 |
| ---------------- | ---------------------------------------------------- | ----------------------- |
| `main_branch`    | Branch to merge into                                 | Auto-detected           |
| `worktree_dir`   | Directory for worktrees (absolute or relative)       | `<project>__worktrees/` |
| `window_prefix`  | Prefix for tmux window names                         | `wm-`                   |
| `agent`          | Default agent for `<agent>` placeholder              | `claude`                |
| `merge_strategy` | Default merge strategy (`merge`, `rebase`, `squash`) | `merge`                 |

#### Naming options

| Option            | Description                                 | Default |
| ----------------- | ------------------------------------------- | ------- |
| `worktree_naming` | How to derive names from branches           | `full`  |
| `worktree_prefix` | Prefix for worktree directories and windows | none    |

`worktree_naming` strategies:

- `full`: Use the full branch name (slashes become dashes)
- `basename`: Use only the part after the last `/` (e.g., `prj-123/feature` →
  `feature`)

#### Panes

Define your tmux pane layout with the `panes` array:

```yaml
panes:
  - command: <agent>
    focus: true
  - command: npm run dev
    split: horizontal
    size: 15
```

Each pane supports:

| Option       | Description                                         | Default |
| ------------ | --------------------------------------------------- | ------- |
| `command`    | Command to run (use `<agent>` for configured agent) | Shell   |
| `focus`      | Whether this pane receives focus                    | `false` |
| `split`      | Split direction (`horizontal` or `vertical`)        | —       |
| `size`       | Absolute size in lines/cells                        | 50%     |
| `percentage` | Size as percentage (1-100)                          | 50%     |

**Note**: The `<agent>` placeholder must be the entire command value to be
substituted. To add extra flags, either include them in the `agent` config
(e.g., `agent: "claude --verbose"`) or use the literal command name (e.g.,
`command: "claude --verbose"`).

#### File operations

Copy or symlink files into new worktrees:

```yaml
files:
  copy:
    - .env
  symlink:
    - node_modules
    - .pnpm-store
```

Both `copy` and `symlink` accept glob patterns.

#### Lifecycle hooks

Run commands at specific points in the worktree lifecycle. All hooks run with
the **worktree directory** as the working directory and receive environment
variables: `WM_HANDLE`, `WM_WORKTREE_PATH`, `WM_PROJECT_ROOT`.

| Hook          | When it runs                                      | Additional env vars                  |
| ------------- | ------------------------------------------------- | ------------------------------------ |
| `post_create` | After worktree creation, before tmux window opens | —                                    |
| `pre_merge`   | Before merging (aborts on failure)                | `WM_BRANCH_NAME`, `WM_TARGET_BRANCH` |
| `pre_remove`  | Before worktree removal (aborts on failure)       | —                                    |

Example:

```yaml
post_create:
  - direnv allow

pre_merge:
  - just check
```

#### Agent status icons

Customize the icons shown in tmux window names:

```yaml
status_icons:
  working: '🤖' # Agent is processing
  waiting: '💬' # Agent needs input (auto-clears on focus)
  done: '✅' # Agent finished (auto-clears on focus)
```

Set `status_format: false` to disable automatic tmux format modification

#### Default behavior

- Worktrees are created in `<project>__worktrees` as a sibling directory to your
  project by default
- If no `panes` configuration is defined, workmux provides opinionated defaults:
  - For projects with a `CLAUDE.md` file: Opens the configured agent (see
    `agent` option) in the first pane, defaulting to `claude` if none is set.
  - For all other projects: Opens your default shell.
  - Both configurations include a second pane split horizontally
- `post_create` commands are optional and only run if you configure them

### Automatic setup with panes

Use the `panes` configuration to automate environment setup. Unlike
`post_create` hooks which must finish before the tmux window opens, pane
commands execute immediately _within_ the new window.

This can be used for:

- **Installing dependencies**: Run `npm install` or `cargo build` in a focused
  pane to monitor progress.
- **Starting services**: Launch dev servers, database containers, or file
  watchers automatically.
- **Running agents**: Initialize AI agents with specific context.

Since these run in standard tmux panes, you can interact with them (check logs,
restart servers) just like a normal terminal session.

Running dependency installation (like `pnpm install`) in a pane command rather
than `post_create` has a key advantage: you get immediate access to the tmux
window while installation runs in the background. With `post_create`, you'd have
to wait for the install to complete before the window even opens. This also
means AI agents can start working immediately in their pane while dependencies
install in parallel.

```yaml
panes:
  # Pane 1: Install dependencies, then start dev server
  - command: pnpm install && pnpm run dev

  # Pane 2: AI agent
  - command: <agent>
    split: horizontal
    focus: true
```

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

Each worktree is a separate working directory for a different branch, all
sharing the same git repository. This allows you to work on multiple branches
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
- [`remove`](#workmux-remove-name-alias-rm) - Remove worktrees without merging
- [`list`](#workmux-list) - List all worktrees with status
- [`open`](#workmux-open-name) - Open a tmux window for an existing worktree
- [`close`](#workmux-close-name) - Close a worktree's tmux window (keeps
  worktree)
- [`path`](#workmux-path-name) - Get the filesystem path of a worktree
- [`dashboard`](#workmux-dashboard) - Show TUI dashboard of all active agents
- [`init`](#workmux-init) - Generate configuration file
- [`claude prune`](#workmux-claude-prune) - Clean up stale Claude Code entries
- [`completions`](#workmux-completions-shell) - Generate shell completions
- [`docs`](#workmux-docs) - Show detailed documentation

### `workmux add <branch-name>`

Creates a new git worktree with a matching tmux window and switches you to it
immediately. If the branch doesn't exist, it will be created automatically.

- `<branch-name>`: Name of the branch to create or switch to, a remote branch
  reference (e.g., `origin/feature-branch`), or a GitHub fork reference (e.g.,
  `user:branch`). Remote and fork references are automatically fetched and
  create a local branch with the derived name. Optional when using `--pr`.

#### Options

- `--base <branch|commit|tag>`: Specify a base branch, commit, or tag to branch
  from when creating a new branch. By default, new branches are created from the
  current branch you have checked out.
- `--pr <number>`: Checkout a GitHub pull request by its number into a new
  worktree.
  - Requires the `gh` command-line tool to be installed and authenticated.
  - The local branch name defaults to the PR's head branch name, but can be
    overridden (e.g., `workmux add custom-name --pr 123`).
- `-A, --auto-name`: Generate branch name from prompt using LLM. See
  [Automatic branch name generation](#automatic-branch-name-generation).
- `--name <name>`: Override the worktree directory and tmux window name. By
  default, these are derived from the branch name (slugified). Cannot be used
  with multi-worktree generation (`--count`, `--foreach`, or multiple
  `--agent`).
- `-b, --background`: Create the tmux window in the background without switching
  to it. Useful with `--prompt-editor`.
- `-w, --with-changes`: Move uncommitted changes from the current worktree to
  the new worktree, then reset the original worktree to a clean state. Useful
  when you've started working on main and want to move your branches to a new
  worktree.
- `--patch`: Interactively select which changes to move (requires
  `--with-changes`). Opens an interactive prompt for selecting hunks to stash.
- `-u, --include-untracked`: Also move untracked files (requires
  `--with-changes`). By default, only staged and modified tracked files are
  moved.
- `-p, --prompt <text>`: Provide an inline prompt that will be automatically
  passed to AI agent panes.
- `-P, --prompt-file <path>`: Provide a path to a file whose contents will be
  used as the prompt.
- `-e, --prompt-editor`: Open your `$EDITOR` (or `$VISUAL`) to write the prompt
  interactively.
- `-a, --agent <name>`: The agent(s) to use for the worktree(s). Can be
  specified multiple times to generate a worktree for each agent. Overrides the
  `agent` from your config file.
- `-W, --wait`: Block until the created tmux window is closed. Useful for
  scripting when you want to wait for an agent to complete its work. The agent
  can signal completion by running `workmux remove --keep-branch`.

#### Skip options

These options allow you to skip expensive setup steps when they're not needed
(e.g., for documentation-only changes):

- `-H, --no-hooks`: Skip running `post_create` commands
- `-F, --no-file-ops`: Skip file copy/symlink operations (e.g., skip linking
  `node_modules`)
- `-C, --no-pane-cmds`: Skip executing pane commands (panes open with plain
  shells instead)

#### What happens

1. Determines the **handle** for the worktree by slugifying the branch name
   (e.g., `feature/auth` becomes `feature-auth`). This can be overridden with
   the `--name` flag.
2. Creates a git worktree at `<worktree_dir>/<handle>` (the `worktree_dir` is
   configurable and defaults to a sibling directory of your project)
3. Runs any configured file operations (copy/symlink)
4. Executes `post_create` commands if defined (runs before the tmux window
   opens, so keep them fast)
5. Creates a new tmux window named `<window_prefix><handle>` (e.g.,
   `wm-feature-auth` with `window_prefix: wm-`)
6. Sets up your configured tmux pane layout
7. Automatically switches your tmux client to the new window

#### Examples

##### Basic usage

```bash
# Create a new branch and worktree
workmux add user-auth

# Use an existing branch
workmux add existing-work

# Create a new branch from a specific base
workmux add hotfix --base production

# Create a worktree from a remote branch (creates local branch "user-auth-pr")
workmux add origin/user-auth-pr

# Remote branches with slashes work too (creates local branch "feature/foo")
workmux add origin/feature/foo

# Create a worktree in the background without switching to it
workmux add feature/parallel-task --background

# Use a custom name for the worktree directory and tmux window
workmux add feature/long-descriptive-branch-name --name short
```

##### Checking out pull requests and fork branches

```bash
# Checkout PR #123. The local branch will be named after the PR's branch.
workmux add --pr 123

# Checkout PR #456 with a custom local branch name
workmux add fix/api-bug --pr 456

# Checkout a fork branch using GitHub's owner:branch format (copy from GitHub UI)
workmux add someuser:feature-branch
```

##### Moving changes to a new worktree

```bash
# Move uncommitted changes to a new worktree (including untracked files)
workmux add feature/new-thing --with-changes -u

# Move only staged/modified files (not untracked files)
workmux add fix/bug --with-changes

# Interactively select which changes to move
workmux add feature/partial --with-changes --patch
```

##### AI agent prompts

```bash
# Create a worktree with an inline prompt for AI agents
workmux add feature/ai --prompt "Implement user authentication with OAuth"

# Override the default agent for a specific worktree
workmux add feature/testing -a gemini

# Create a worktree with a prompt from a file
workmux add feature/refactor --prompt-file task-description.md

# Open your editor to write a prompt interactively
workmux add feature/new-api --prompt-editor
```

##### Skipping setup steps

```bash
# Skip expensive setup for documentation-only changes
workmux add docs-update --no-hooks --no-file-ops --no-pane-cmds

# Skip just the file operations (e.g., you don't need node_modules)
workmux add quick-fix --no-file-ops
```

##### Scripting with --wait

```bash
# Block until the agent completes and closes the window
workmux add feature/api --wait -p "Implement the REST API, then run: workmux remove --keep-branch"

# Use in a script to run sequential agent tasks
for task in task1.md task2.md task3.md; do
  workmux add "task-$(basename $task .md)" --wait -P "$task"
done
```

#### AI agent integration

When you provide a prompt via `--prompt`, `--prompt-file`, or `--prompt-editor`,
workmux automatically injects the prompt into panes running the configured agent
command (e.g., `claude`, `codex`, `opencode`, `gemini`, or whatever you've set
via the `agent` config or `--agent` flag) without requiring any `.workmux.yaml`
changes:

- Panes with a command matching the configured agent are automatically started
  with the given prompt.
- You can keep your `.workmux.yaml` pane configuration simple (e.g.,
  `panes: [{ command: "<agent>" }]`) and let workmux handle prompt injection at
  runtime.

This means you can launch AI agents with task-specific prompts without modifying
your project configuration for each task.

#### Automatic branch name generation

The `--auto-name` (`-A`) flag generates a branch name from your prompt using an
LLM via the [`llm`](https://llm.datasette.io/) CLI tool.

##### Usage

```bash
# Opens editor for prompt, generates branch name
workmux add -A

# With inline prompt
workmux add -A -p "Add OAuth authentication"

# With prompt file
workmux add -A -P task-spec.md
```

##### Requirements

Install the `llm` CLI tool:

```bash
pipx install llm
```

Configure a model (e.g., OpenAI):

```bash
llm keys set openai
# Or use a local model
llm install llm-ollama
```

##### Configuration

Optionally specify a model and/or custom system prompt in `.workmux.yaml`:

```yaml
auto_name:
  model: 'gemini-2.5-flash-lite'
  system_prompt: |
    Generate a concise git branch name based on the task description.

    Rules:
    - Use kebab-case (lowercase with hyphens)
    - Keep it short: 1-3 words, max 4 if necessary
    - Focus on the core task/feature, not implementation details
    - No prefixes like feat/, fix/, chore/

    Examples of good branch names:
    - "Add dark mode toggle" → dark-mode
    - "Fix the search results not showing" → fix-search
    - "Refactor the authentication module" → auth-refactor
    - "Add CSV export to reports" → export-csv
    - "Shell completion is broken" → shell-completion

    Output ONLY the branch name, nothing else.
```

If `model` is not configured, uses `llm`'s default model.

Recommended models for fast, cheap branch name generation:

- `gemini-2.5-flash-lite` (recommended)
- `gpt-5-nano`

#### Parallel workflows & multi-worktree generation

workmux can generate multiple worktrees from a single `add` command, which is
ideal for running parallel experiments or delegating tasks to multiple AI
agents. This is controlled by four mutually exclusive modes:

- (`-a`, `--agent`): Create a worktree for each specified agent.
- (`-n`, `--count`): Create a specific number of worktrees.
- (`--foreach`): Create worktrees based on a matrix of variables.
- **stdin**: Pipe input lines to create worktrees with templated prompts.

When using any of these modes, branch names are generated from a template, and
prompts can be templated with variables.

##### Multi-worktree options

- `-a, --agent <name>`: When used multiple times, creates one worktree for each
  agent.
- `-n, --count <number>`: Creates `<number>` worktree instances. Can be combined
  with a single `--agent` flag to apply that agent to all instances.
- `--foreach <matrix>`: Creates worktrees from a variable matrix string. The
  format is `"var1:valA,valB;var2:valX,valY"`. All value lists must have the
  same length. Values are paired by index position (zip, not Cartesian product):
  the first value of each variable goes together, the second with the second,
  etc.
- `--branch-template <template>`: A
  [MiniJinja](https://docs.rs/minijinja/latest/minijinja/) (Jinja2-compatible)
  template for generating branch names.
  - Available variables: `{{ base_name }}`, `{{ agent }}`, `{{ num }}`,
    `{{ index }}`, `{{ input }}` (stdin), and any variables from `--foreach`.
  - Default:
    `{{ base_name }}{% if agent %}-{{ agent | slugify }}{% endif %}{% for key, value in foreach_vars %}-{{ value | slugify }}{% endfor %}{% if num %}-{{ num }}{% endif %}`
- `--max-concurrent <number>`: Limits how many worktrees run simultaneously.
  When set, workmux creates up to `<number>` worktrees, then waits for any
  window to close before starting the next. Requires agents to close windows
  when done (e.g., via prompt instruction to run
  `workmux remove --keep-branch`).

##### Prompt templating

When generating multiple worktrees, any prompt provided via `-p`, `-P`, or `-e`
is treated as a MiniJinja template. You can use variables from your generation
mode to create unique prompts for each agent or instance.

##### Variable matrices in prompt files

Instead of passing `--foreach` on the command line, you can specify the variable
matrix directly in your prompt file using YAML frontmatter. This is more
convenient for complex matrices and keeps the variables close to the prompt that
uses them.

**Format:**

Create a prompt file with YAML frontmatter at the top, separated by `---`:

**Example 1:** `mobile-task.md`

```markdown
---
foreach:
  platform: [iOS, Android]
  lang: [swift, kotlin]
---

Build a {{ platform }} app using {{ lang }}. Implement user authentication and
data persistence.
```

```bash
workmux add mobile-app --prompt-file mobile-task.md
# Generates worktrees: mobile-app-ios-swift, mobile-app-android-kotlin
```

**Example 2:** `agent-task.md` (using `agent` as a foreach variable)

```markdown
---
foreach:
  agent: [claude, gemini]
---

Implement the dashboard refactor using your preferred approach.
```

```bash
workmux add refactor --prompt-file agent-task.md
# Generates worktrees: refactor-claude, refactor-gemini
```

**Behavior:**

- Variables from the frontmatter are available in both the prompt template and
  the branch name template
- All value lists must have the same length, and values are paired by index
  position (same zip behavior as `--foreach`)
- CLI `--foreach` overrides frontmatter with a warning if both are present
- Works with both `--prompt-file` and `--prompt-editor`

##### Stdin input

You can pipe input lines to `workmux add` to create multiple worktrees. Each
line becomes available as the `{{ input }}` template variable in your prompt.
This is useful for batch-processing tasks from external sources.

**Plain text:** Each line becomes `{{ input }}`

```bash
echo -e "api\nauth\ndatabase" | workmux add refactor -P task.md
# {{ input }} = "api", "auth", "database"
```

**JSON lines:** Each key becomes a template variable

```bash
gh repo list --json url,name --jq -c '.[]' | workmux add analyze \
  --branch-template '{{ base_name }}-{{ name }}' \
  -P prompt.md
# Line: {"url":"https://github.com/raine/workmux","name":"workmux"}
# Variables: {{ url }}, {{ name }}, {{ input }} (raw JSON line)
```

This lets you structure data upstream with `jq` and use meaningful branch names
while keeping the full URL available in your prompt.

**Behavior:**

- Empty lines and whitespace-only lines are filtered out
- Stdin input cannot be combined with `--foreach` (mutually exclusive)
- JSON objects (lines starting with `{`) are parsed and each key becomes a
  variable
- `{{ input }}` always contains the raw line
- If JSON contains an `input` key, it overwrites the raw line value

##### Examples

```bash
# Create one worktree for claude and one for gemini with a focused prompt
workmux add my-feature -a claude -a gemini -p "Implement the new search API integration"
# Generates worktrees: my-feature-claude, my-feature-gemini

# Create 2 instances of the default agent
workmux add my-feature -n 2 -p "Implement task #{{ num }} in TASKS.md"
# Generates worktrees: my-feature-1, my-feature-2

# Create worktrees from a variable matrix
workmux add my-feature --foreach "platform:iOS,Android" -p "Build for {{ platform }}"
# Generates worktrees: my-feature-ios, my-feature-android

# Create agent-specific worktrees via --foreach
workmux add my-feature --foreach "agent:claude,gemini" -p "Implement the dashboard refactor"
# Generates worktrees: my-feature-claude, my-feature-gemini

# Use frontmatter in a prompt file for cleaner syntax
# task.md contains:
# ---
# foreach:
#   env: [staging, production]
#   task: [smoke-tests, integration-tests]
# ---
# Run {{ task }} against the {{ env }} environment
workmux add testing --prompt-file task.md
# Generates worktrees: testing-staging-smoke-tests, testing-production-integration-tests

# Pipe input from stdin to create worktrees
# review.md contains: Review the {{ input }} module for security issues.
echo -e "auth\npayments\napi" | workmux add review -A -P review.md
# Generates worktrees with LLM-generated branch names for each module
```

##### Recipe: Batch processing with worker pools

Combine stdin input, prompt templating, and concurrency limits to create a
worker pool that processes items from an external command.

**Example: Generate test scaffolding for untested files**

```bash
# generate-tests.md contains:
# Read the file at {{ input }} and generate a test suite covering
# the exported functions. Focus on happy path and edge cases.
# When done, run: workmux remove --keep-branch

find src/utils -name "*.ts" ! -name "*.test.ts" | \
  workmux add add-tests \
    --branch-template '{{ base_name }}-{{ index }}' \
    --prompt-file generate-tests.md \
    --max-concurrent 3 \
    --background
```

- `find ...` lists files without tests (one per line) piped to stdin
- `--branch-template` uses `{{ index }}` for unique branch names
- `--prompt-file` uses `{{ input }}` to pass each file path to the agent
- `--max-concurrent 3` limits parallel agents to avoid rate limits
- `--background` runs without switching focus

---

### `workmux merge [branch-name]`

Merges a branch into a target branch (main by default) and automatically cleans
up all associated resources (worktree, tmux window, and local branch).

- `[branch-name]`: Optional name of the branch to merge. If omitted,
  automatically detects the current branch from the worktree you're in.

#### Options

- `--into <branch>`: Merge into the specified branch instead of the main branch.
  Useful for stacked PRs, git-flow workflows, or merging subtasks into a parent
  feature branch. If the target branch has its own worktree, the merge happens
  there; otherwise, the main worktree is used.
- `--ignore-uncommitted`: Commit any staged changes before merging without
  opening an editor
- `--keep`, `-k`: Keep the worktree, window, and branch after merging (skip
  cleanup). Useful when you want to verify the merge before cleaning up.

#### Merge strategies

By default, `workmux merge` performs a standard merge commit (configurable via
`merge_strategy`). You can override the configured behavior with these mutually
exclusive flags:

- `--rebase`: Rebase the feature branch onto the target before merging (creates
  a linear history via fast-forward merge). If conflicts occur, you'll need to
  resolve them manually in the worktree and run `git rebase --continue`.
- `--squash`: Squash all commits from the feature branch into a single commit on
  the target. You'll be prompted to provide a commit message in your editor.

If you don't want to have merge commits in your main branch, use the `rebase`
merge strategy, which does `--rebase` by default.

```yaml
# ~/.config/workmux/config.yaml
merge_strategy: rebase
```

#### What happens

1. Determines which branch to merge (specified branch or current branch if
   omitted)
2. Determines the target branch (`--into` or main branch from config)
3. Checks for uncommitted changes (errors if found, unless
   `--ignore-uncommitted` is used)
4. Commits staged changes if present (unless `--ignore-uncommitted` is used)
5. Merges your branch into the target using the selected strategy (default:
   merge commit)
6. Deletes the tmux window (including the one you're currently in if you ran
   this from a worktree) — skipped if `--keep` is used
7. Removes the worktree — skipped if `--keep` is used
8. Deletes the local branch — skipped if `--keep` is used

#### Typical workflow

When you're done working in a worktree, simply run `workmux merge` from within
that worktree's tmux window. The command will automatically detect which branch
you're on, merge it into main, and close the current window as part of cleanup.

#### Examples

```bash
# Merge branch into main (default: merge commit)
workmux merge user-auth

# Merge the current worktree you're in
# (run this from within the worktree's tmux window)
workmux merge

# Rebase onto main before merging for a linear history
workmux merge user-auth --rebase

# Squash all commits into a single commit
workmux merge user-auth --squash

# Merge but keep the worktree/window/branch to verify before cleanup
workmux merge user-auth --keep
# ... verify the merge in main ...
workmux remove user-auth  # clean up later when ready

# Merge into a different branch (stacked PRs)
workmux merge feature/subtask --into feature/parent
```

---

### `workmux remove [name]...` (alias: `rm`)

Removes worktrees, tmux windows, and branches without merging (unless you keep
the branches). Useful for abandoning work or cleaning up experimental branches.
Supports removing multiple worktrees in a single command.

- `[name]...`: One or more worktree names (the directory names). Defaults to
  current directory name if omitted.

#### Options

- `--all`: Remove all worktrees at once (except the main worktree). Prompts for
  confirmation unless `--force` is used. Safely skips worktrees with uncommitted
  changes or unmerged commits.
- `--gone`: Remove worktrees whose upstream remote branch has been deleted
  (e.g., after a PR is merged on GitHub). Automatically runs `git fetch --prune`
  first.
- `--force`, `-f`: Skip confirmation prompt and ignore uncommitted changes
- `--keep-branch`, `-k`: Remove only the worktree and tmux window while keeping
  the local branch

#### Examples

```bash
# Remove the current worktree (run from within the worktree)
workmux remove

# Remove a specific worktree with confirmation if unmerged
workmux remove experiment

# Remove multiple worktrees at once
workmux rm feature-a feature-b feature-c

# Remove multiple worktrees with force (no confirmation)
workmux rm -f old-work stale-branch

# Use the alias
workmux rm old-work

# Remove worktree/window but keep the branch
workmux remove --keep-branch experiment

# Force remove without prompts
workmux rm -f experiment

# Remove worktrees whose remote branches were deleted (e.g., after PR merge)
workmux rm --gone

# Force remove all gone worktrees (no confirmation)
workmux rm --gone -f

# Remove all worktrees at once
workmux rm --all
```

---

### `workmux list` (alias: `ls`)

Lists all git worktrees with their tmux window status and merge status.

#### Options

- `--pr`: Show GitHub PR status for each worktree. Requires the `gh` CLI to be
  installed and authenticated. Note that it shows pull requests' statuses with
  [Nerd Font](https://www.nerdfonts.com/) icons, which requires Nerd Font
  compatible font installed.

#### Examples

```bash
# List all worktrees
workmux list

# List with PR status
workmux list --pr
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

---

### `workmux open <name>`

Opens or switches to a tmux window for a pre-existing git worktree. If the
window already exists, switches to it. If not, creates a new window with the
configured pane layout and environment.

- `<name>`: Worktree name (the directory name, which is also the tmux window
  name without the prefix). This is the name you see in your tmux window list.

#### Options

- `-n, --new`: Force opening in a new window even if one already exists. Creates
  a duplicate window with a suffix (e.g., `-2`, `-3`). Useful for having
  multiple terminal views into the same worktree.
- `--run-hooks`: Re-runs the `post_create` commands (these block window
  creation).
- `--force-files`: Re-applies file copy/symlink operations. Useful for restoring
  a deleted `.env` file.
- `-p, --prompt <text>`: Provide an inline prompt for AI agent panes.
- `-P, --prompt-file <path>`: Provide a path to a file containing the prompt.
- `-e, --prompt-editor`: Open your editor to write the prompt interactively.

#### What happens

1. Verifies that a worktree with `<name>` exists.
2. If a tmux window exists and `--new` is not set, switches to it.
3. Otherwise, creates a new tmux window (with suffix if duplicating).
4. (If specified) Runs file operations and `post_create` hooks.
5. Sets up your configured tmux pane layout.
6. Automatically switches your tmux client to the new window.

#### Examples

```bash
# Open or switch to a window for an existing worktree
workmux open user-auth

# Force open a second window for the same worktree (creates user-auth-2)
workmux open user-auth --new

# Open with a prompt for AI agents
workmux open user-auth -p "Continue implementing the login flow"

# Open and re-run dependency installation
workmux open user-auth --run-hooks

# Open and restore configuration files
workmux open user-auth --force-files
```

---

### `workmux close [name]`

Closes the tmux window for a worktree without removing the worktree or branch.
This is useful when you want to temporarily close a window to reduce clutter or
free resources, but plan to return to the work later.

- `[name]`: Optional worktree name (the directory name). Defaults to current
  directory if omitted.

#### Examples

```bash
# Close the window for a specific worktree
workmux close user-auth

# Close the current worktree's window (run from within the worktree)
workmux close
```

To reopen the window later, use [`workmux open`](#workmux-open-name).

**Tip**: You can also use tmux's native kill-window command (default:
`prefix + &`) to close a worktree's window with the same effect.

---

### `workmux path <name>`

Prints the filesystem path of an existing worktree. Useful for scripting or
quickly navigating to a worktree directory.

- `<name>`: Worktree name (the directory name).

#### Examples

```bash
# Get the path of a worktree
workmux path user-auth
# Output: /Users/you/project__worktrees/user-auth

# Use in scripts or with cd
cd "$(workmux path user-auth)"

# Copy a file to a worktree
cp config.json "$(workmux path feature-branch)/"
```

---

### `workmux dashboard`

Opens a TUI dashboard showing all active AI agents across all tmux sessions.
Useful for monitoring multiple parallel agents and quickly jumping between them.

> [!IMPORTANT]
> This feature requires [agent status tracking](#agent-status-tracking) to be
> configured. Without it, no agents will appear in the dashboard.

![workmux dashboard](https://raw.githubusercontent.com/raine/workmux/refs/heads/main/meta/dashboard.webp)

#### Keybindings

| Key       | Action                              |
| --------- | ----------------------------------- |
| `1`-`9`   | Quick jump to agent (closes dashboard) |
| `p`       | Peek at agent (dashboard stays open)   |
| `s`       | Cycle sort mode                        |
| `i`       | Enter input mode (type to agent)       |
| `Ctrl+u`  | Scroll preview up                      |
| `Ctrl+d`  | Scroll preview down                    |
| `Enter`   | Go to selected agent (closes dashboard)|
| `j`/`k`   | Navigate up/down                    |
| `q`/`Esc` | Quit                                |

The bottom half shows a live preview of the selected agent's terminal output.
Press `i` to enter input mode and type directly to the agent without leaving the
dashboard.

#### Sort modes

Press `s` to cycle through sort modes:

- **Priority** (default): Waiting > Done > Working > Stale
- **Project**: Group by project name, then by priority within each project
- **Recency**: Most recently updated first
- **Natural**: Original tmux order (by pane creation)

Your sort preference persists in the tmux session.

#### Columns

- **#**: Quick jump key (1-9)
- **Project**: Project name (from `__worktrees` path or directory name)
- **Agent**: Worktree/window name
- **Title**: Claude Code session title (auto-generated summary)
- **Status**: Agent status icon (🤖 working, 💬 waiting, ✅ done, or "stale")
- **Duration**: Time since last status change

#### Example tmux binding

Add to your `~/.tmux.conf` for quick access:

```bash
bind C-s display-popup -h 15 -w 100 -E "workmux dashboard"
```

Then press `prefix + Ctrl-s` to open the dashboard as a tmux popup.

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

---

### `workmux docs`

Displays this README with terminal formatting. Useful for quick reference
without leaving the terminal.

When run interactively, renders markdown with colors and uses a pager (`less`).
When piped (e.g., to an LLM), outputs raw markdown for clean context.

#### Using with AI agents

You can ask an agent to read the docs and configure workmux for you:

```
> run `workmux docs` and configure workmux so that on the left pane
  there is claude as agent, and on the right side neovim and empty
  shell on top of each other

⏺ Bash(workmux docs)
  ⎿  <p align="center">
       <picture>
     … +923 lines

⏺ Write(.workmux.yaml)
  ⎿  Wrote 9 lines to .workmux.yaml

⏺ Created .workmux.yaml with the layout:
  - Left: claude agent (focused)
  - Right top: neovim
  - Right bottom: empty shell
```

## Agent status tracking

Workmux can display the status of the agent in your tmux window list, giving you
at-a-glance visibility into what the agent in each window doing.

![tmux status showing agent icons](https://raw.githubusercontent.com/raine/workmux/refs/heads/main/meta/status.webp)

#### Key

- 🤖 = agent is working
- 💬 = agent is waiting for user input
- ✅ = agent finished (auto-clears on window focus)

**Note**: Currently only Claude Code seems to support hooks that enable this
kind of functionality. Gemini's support is
[on the way](https://github.com/google-gemini/gemini-cli/issues/9070). Codex
support can tracked in
[this issue](https://github.com/openai/codex/issues/2109).

### Setup

Install the workmux status plugin in Claude Code:

```
claude plugin marketplace add raine/workmux
claude plugin install workmux-status
```

Alternatively, you can manually add the hooks to `~/.claude/settings.json`. See
[.claude-plugin/plugin.json](.claude-plugin/plugin.json) for the hook
configuration.

Workmux automatically modifies your tmux `window-status-format` to display the
status icons. This happens once per session and only affects the current tmux
session (not your global config).

### Customization

You can customize the icons in your config:

```yaml
# ~/.config/workmux/config.yaml
status_icons:
  working: '🔄'
  waiting: '⏸️'
  done: '✔️'
```

If you prefer to manage the tmux format yourself, disable auto-modification and
add the status variable to your `~/.tmux.conf`:

```yaml
# ~/.config/workmux/config.yaml
status_format: false
```

```bash
# ~/.tmux.conf
set -g window-status-format '#I:#W#{?@workmux_status, #{@workmux_status},}#{?window_flags,#{window_flags}, }'
set -g window-status-current-format '#I:#W#{?@workmux_status, #{@workmux_status},}#{?window_flags,#{window_flags}, }'
```

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

workmux turns a multi-step manual workflow into two simple commands, making
parallel development workflows practical.

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

## Why git worktrees?

[Git worktrees](https://git-scm.com/docs/git-worktree) let you have multiple
branches checked out at once in the same repository, each in a separate
directory. This provides two main advantages over a standard single-directory
setup:

- **Painless context switching**: Switch between tasks just by changing
  directories (`cd ../other-branch`). There's no need to `git stash` or make
  temporary commits. Your work-in-progress, editor state, and command history
  remain isolated and intact for each branch.

- **True parallel development**: Work on multiple branches simultaneously
  without interference. You can run builds, install dependencies
  (`npm install`), or run tests in one worktree while actively coding in
  another. This isolation is perfect for running multiple AI agents in parallel
  on different tasks.

In a standard Git setup, switching branches disrupts your flow by requiring a
clean working tree. Worktrees remove this friction. `workmux` automates the
entire process and pairs each worktree with a dedicated tmux window, creating
fully isolated development environments. See [Why workmux?](#why-workmux) for
how workmux streamlines this workflow.

## Git worktree caveats

While powerful, git worktrees have nuances that are important to understand.
workmux is designed to automate solutions to these, but awareness of the
underlying mechanics helps.

- [Gitignored files require configuration](#gitignored-files-require-configuration)
- [Conflicts](#conflicts)
- [Package manager considerations (pnpm, yarn)](#package-manager-considerations-pnpm-yarn)
- [Rust projects](#rust-projects)
- [Symlinks and `.gitignore` trailing slashes](#symlinks-and-gitignore-trailing-slashes)
- [Local git ignores (`.git/info/exclude`) are not shared](#local-git-ignores-gitinfoexclude-are-not-shared)

### Gitignored files require configuration

When `git worktree add` creates a new working directory, it's a clean checkout.
Files listed in your `.gitignore` (e.g., `.env` files, `node_modules`, IDE
configuration) will not exist in the new worktree by default. Your application
will be broken in the new worktree until you manually create or link these
necessary files.

This is a primary feature of workmux. Use the `files` section in your
`.workmux.yaml` to automatically copy or symlink these files on creation:

```yaml
# .workmux.yaml
files:
  copy:
    - .env # Copy environment variables
  symlink:
    - .next/cache # Share Next.js build cache
```

Note: Symlinking `node_modules` can be efficient but only works if all worktrees
share identical dependencies. If different branches have different dependency
versions, each worktree needs its own installation. For dependency installation,
consider using a pane command instead of `post_create` hooks - this runs the
install in the background without blocking the worktree and window creation:

```yaml
panes:
  - command: npm install
    focus: true
  - split: horizontal
```

### Conflicts

Worktrees isolate your filesystem, but they do not prevent merge conflicts. If
you modify the area of code on two different branches (in two different
worktrees), you will still have a conflict when you merge one into the other.

The best practice is to work on logically separate features in parallel
worktrees. When conflicts are unavoidable, use standard git tools to resolve
them. You can also leverage an AI agent within the worktree to assist with the
conflict resolution.

### Package manager considerations (pnpm, yarn)

Modern package managers like `pnpm` use a global store with symlinks to
`node_modules`. Each worktree typically needs its own `pnpm install` to set up
the correct dependency versions for that branch.

If your worktrees always have identical dependencies (e.g., working on multiple
features from the same base), you could potentially symlink `node_modules`
between worktrees. However, this breaks as soon as branches diverge in their
dependencies, so it's generally safer to run a fresh install in each worktree.

Note: In large monorepos, cleaning up `node_modules` during worktree removal can
take significant time. workmux has a
[special cleanup mechanism](https://github.com/raine/workmux/blob/main/src/scripts/cleanup_node_modules.sh)
that moves `node_modules` to a temporary location and deletes it in the
background, making the `remove` command return almost instantly.

### Rust projects

Unlike `node_modules`, Rust's `target/` directory should **not** be symlinked
between worktrees. Cargo locks the `target` directory during builds, so sharing
it would block parallel builds and defeat the purpose of worktrees.

Instead, use [sccache](https://github.com/mozilla/sccache) to share compiled
dependencies across worktrees:

```bash
brew install sccache
```

Add to `~/.cargo/config.toml`:

```toml
[build]
rustc-wrapper = "sccache"
```

This caches compiled dependencies globally, so new worktrees benefit from cached
artifacts without any lock contention.

### Symlinks and `.gitignore` trailing slashes

If your `.gitignore` uses a trailing slash to ignore directories (e.g.,
`tests/venv/`), symlinks to that path in the created worktree will **not** be
ignored and will show up in `git status`. This is because `venv/` only matches
directories, not files (symlinks).

To ignore both directories and symlinks, remove the trailing slash:

```diff
- tests/venv/
+ tests/venv
```

### Local git ignores (`.git/info/exclude`) are not shared

The local git ignore file, `.git/info/exclude`, is specific to the main
worktree's git directory and is not respected in other worktrees. Personal
ignore patterns for your editor or temporary files may not apply in new
worktrees, causing them to appear in `git status`.

For personal ignores, use a global git ignore file. For project-specific ignores
that are safe to share with your team, add them to the project's main
`.gitignore` file.

## Tips

### Nerdfont window prefix

If you have a [Nerd Font](https://www.nerdfonts.com/) installed (fonts patched
with icons for developers), you can use the git branch icon as your window
prefix for a cleaner look:

```yaml
# ~/.config/workmux/config.yaml
window_prefix: "\uf418 "
```

![nerdfont window prefix](https://raw.githubusercontent.com/raine/workmux/refs/heads/main/meta/nerdfont-prefix.webp)

### Using direnv

If your project uses [direnv](https://direnv.net/) for environment management,
you can configure workmux to automatically set it up in new worktrees:

```yaml
# .workmux.yaml
post_create:
  - direnv allow

files:
  symlink:
    - .envrc
```

### Claude Code permissions

By default, Claude Code prompts for permission before running commands. There
are several ways to handle this in worktrees:

**Share permissions across worktrees**

To keep permission prompts but share granted permissions across worktrees:

```yaml
files:
  symlink:
    - .claude/settings.local.json
```

Add this to your global config (`~/.config/workmux/config.yaml`) or project's
`.workmux.yaml`. Since this file contains user-specific permissions, also add it
to `.gitignore`:

```
.claude/settings.local.json
```

**Skip permission prompts (yolo mode)**

To skip prompts entirely, either configure the agent with the flag:

```yaml
agent: 'claude --dangerously-skip-permissions'
```

This only affects workmux-created worktrees. Alternatively, use a global shell
alias:

```bash
alias claude="claude --dangerously-skip-permissions"
```

### Delegating tasks with a custom command

📝 **See [this blog post][delegating-post]** for a detailed walkthrough of the
workflow.

A Claude Code [custom slash command][custom slash commands] can streamline task
delegation to worktree agents. Save this as `~/.claude/commands/worktree.md`:

```markdown
Launch one or more tasks in new git worktrees using workmux.

Tasks: $ARGUMENTS

## Instructions

Note: The tasks above may reference something discussed earlier in the
conversation (e.g., "do option 2", "implement the fix we discussed"). Include
all relevant context from the conversation in each prompt you write.

If tasks reference a markdown file (e.g., a plan or spec), re-read the file to
ensure you have the latest version before writing prompts.

For each task:

1. Generate a short, descriptive worktree name (2-4 words, kebab-case)
2. Write a detailed implementation prompt to a temp file
3. Run `workmux add <worktree-name> -b -P <temp-file>` to create the worktree

The prompt file should:

- Include the full task description
- Use RELATIVE paths only (never absolute paths, since each worktree has its own
  root directory)
- Be specific about what the agent should accomplish

## Workflow

Write ALL temp files first, THEN run all workmux commands in parallel.

After creating the worktrees, inform the user which branches were created.
```

Usage:

```
> /worktree Implement user authentication
> /worktree Fix the race condition in handler.go
> /worktree Add dark mode, Implement caching  # multiple tasks
```

[custom slash commands]:
  https://docs.anthropic.com/en/docs/claude-code/tutorials/custom-slash-commands
[delegating-post]: https://raine.dev/blog/git-worktrees-parallel-agents/

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
interfaces, like a TUI or kanban board. In contrast, workmux adheres to its
philosophy that **tmux is the interface**, providing a native tmux experience
for managing parallel workflows without requiring a separate interface to learn.

## Contributing

Thank you for your interest in contributing! Bug reports and feature suggestions
are always welcome via issues.

My goal is to keep the project simple and fun to maintain. I am generally not
interested in reviewing complex PRs, refactors, or major feature additions, as
they turn a fun hobby project into administrative work.

If you have a small fix, feel free to submit it. For anything larger, please
open an issue first. Thanks for understanding.

## Related projects

- [tmux-tools](https://github.com/raine/tmux-tools) — Collection of tmux
  utilities including file picker, smart sessions, and more
- [tmux-file-picker](https://github.com/raine/tmux-file-picker) — Pop up fzf in
  tmux to quickly insert file paths, perfect for AI coding assistants
- [tmux-bro](https://github.com/raine/tmux-bro) — Smart tmux session manager
  that sets up project-specific sessions automatically
- [claude-history](https://github.com/raine/claude-history) — Search and view
  Claude Code conversation history with fzf
- [consult-llm-mcp](https://github.com/raine/consult-llm-mcp) — MCP server that
  lets Claude Code consult stronger AI models (o3, Gemini, GPT-5.1 Codex)
