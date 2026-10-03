<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="meta/logo-dark.svg">
    <img src="meta/logo.svg" alt="muxix icon" width="300">
  </picture>
</p>

<p align="center">
  <strong>Parallel development in tmux* with git worktrees</strong>
</p>

<p align="center">
  <a href="https://muxix.dev/"><strong>📖 Documentation</strong></a> ·
  <a href="#installation">Install</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#commands">Commands</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

---

Opinionated workflow tool for managing
[git worktrees](https://git-scm.com/docs/git-worktree) and tmux windows as
isolated development environments, so several AI agents can work in parallel
without conflict.

**Philosophy**: Build on tools you already use. tmux/zellij/kitty/etc. for
windowing, git for worktrees, your agent for coding - muxix ties them together.

muxix also treats the *agent itself* as something to provision: one declarative
config installs skills, subagents, plugins, MCP servers, hooks and system-prompt
components across Claude Code, Codex, Copilot CLI, Gemini, OpenCode, pi and omp,
so every worktree gets an identically equipped agent — and whatever an agent
cannot take is reported with the reason instead of silently skipped.

<sup><sub>\* Also supports
<a href="https://muxix.dev/guide/kitty">kitty</a>,
<a href="https://muxix.dev/guide/wezterm">WezTerm</a>, and
<a href="https://muxix.dev/guide/zellij">Zellij</a> as alternative
backends.</sub></sup>

> [!NOTE]
> **muxix is a fork of [workmux](https://github.com/raine/workmux) by
> [@raine](https://github.com/raine), heavily modified.** The worktree and tmux
> core is upstream's work and upstream's credit; everything below marked *fork*
> is not in workmux, and the two are no longer config-compatible in both
> directions.
>
> **Added in this fork:** a declarative agent harness (`setup`/`bootstrap`:
> skills, subagents, plugins, MCP servers, hooks, settings patches, prompt
> components, per-agent profiles), seven supported agent CLIs with status hooks,
> a task-graph store (`task`) for
> external harnesses to drive, sandboxed worktrees (container / Lima /
> microVM), org policy provisioning, a project registry, event tracing, and a
> third dashboard tab.
>
> The rename is a clean break: muxix reads `.muxix.yaml` (not `.workmux.yaml`),
> `MUXIX_*` environment variables, and `~/.config/muxix/`. Coming from workmux,
> rename the project file and the config directory. Binary and plugin names
> differ too, so both tools can be installed side by side.

![muxix screenshot](https://raw.githubusercontent.com/lcensies/muxix/refs/heads/main/meta/screenshot_20260329_165534.webp)


## Features

- Create git worktrees with matching tmux windows in a single command (`add`)
- Merge branches and clean up everything (worktree, tmux window, branches) in
  one command (`merge`)
- [Dashboard](#muxix-dashboard) for monitoring agents, reviewing changes, and
  sending commands
- [Sidebar](https://muxix.dev/guide/sidebar/) for a persistent,
  at-a-glance view of all agents across tmux windows
- [Delegate tasks to worktree agents](#delegating-tasks-with-worktree) with the
  `/worktree` skill
- [Display agent status in tmux window names](#agent-status-tracking)
- [Provision the agents themselves](#declarative-agent-harness) from one config:
  skills, subagents, plugins, MCP servers, hooks, settings and prompt components
  across seven agent CLIs (`setup`)
- Automatically set up your preferred tmux pane layout (editor, shell, watchers,
  etc.)
- Run post-creation hooks (install dependencies, setup database, etc.)
- Copy or symlink configuration files (`.env`, `node_modules`) into new
  worktrees
- [Sandbox agents](#sandbox) in containers or VMs for enhanced security
- [Automatic branch name generation](#automatic-branch-name-generation) from
  prompts using LLM
- Shell completions

## Installation

### Install script

```bash
curl -fsSL https://raw.githubusercontent.com/lcensies/muxix/main/scripts/install.sh | bash
```

### From source

```bash
cargo install --git https://github.com/lcensies/muxix
```

<details>
<summary>Other methods (Homebrew, crates.io, mise, Nix)</summary>

> The Homebrew tap and the crates.io release publish with the first tagged
> version; until then, use the git install above.

**Homebrew** (macOS/Linux):

```bash
brew install lcensies/muxix/muxix
```

**Cargo** (requires [rustup](https://rustup.rs/)):

```bash
cargo install muxix
```

**mise:**

```bash
mise use -g cargo:lcensies/muxix
```

**Nix** ([flake and home-manager setup](https://muxix.dev/guide/nix)):

```bash
nix profile install github:lcensies/muxix
```

</details>

---

For manual installation, see
[pre-built binaries](https://github.com/lcensies/muxix/releases/latest).

## Quick start

<!-- prettier-ignore -->
> [!NOTE]
> muxix requires a terminal multiplexer. Make sure you have
> [tmux](https://github.com/tmux/tmux) (or
> [WezTerm](https://muxix.dev/guide/wezterm) /
> [Kitty](https://muxix.dev/guide/kitty) /
> [Zellij](https://muxix.dev/guide/zellij)) installed and running
> before you start. See [My tmux setup](https://raine.dev/blog/my-tmux-setup/)
> if you need a starting point.

1. **Initialize configuration (optional)**:

   ```bash
   muxix init
   ```

   This creates a `.muxix.yaml` file to customize your workflow (pane layouts,
   setup commands, file operations, etc.). muxix works out of the box with
   sensible defaults, so this step is optional.

2. **Create a new worktree and tmux window**:

   ```bash
   muxix add new-feature
   ```

   This will:
   - Create a git worktree at
     `<project_root>/../<project_name>__worktrees/new-feature`
   - Copy config files and symlink dependencies (if
     [configured](#file-operations))
   - Run any [`post_create`](#lifecycle-hooks) setup commands
   - Create a tmux window named `wm-new-feature` (the prefix is configurable)
   - Set up your configured or the default tmux pane layout
   - Automatically switch your tmux client to the new window

3. **Do your thing**

4. **Finish and clean up**

   **Local merge:** Run `muxix merge` to merge into the base branch and clean
   up in one step.

   **PR workflow:** Push and open a PR. After it's merged, run `muxix remove`
   to clean up.

## Configuration

muxix uses a two-level configuration system:

- **Global** (`~/.config/muxix/config.yaml`): Personal defaults for all
  projects
- **Project** (`.muxix.yaml`): Project-specific overrides

Project settings override global settings. When you run muxix from a
subdirectory, it walks upward to find the nearest `.muxix.yaml`, allowing
nested configs for monorepos. See the
[Monorepos guide](https://muxix.dev/guide/monorepos#nested-configuration)
for details. For `post_create` and file operation lists (`files.copy`,
`files.symlink`), you can use `"<global>"` to include global values alongside
project-specific ones. Other settings like `panes` are replaced entirely when
defined in the project config.

### Global configuration example

`~/.config/muxix/config.yaml`:

```yaml
nerdfont: true # Enable nerdfont icons (prompted on first run)
merge_strategy: rebase # Make muxix merge do rebase by default
merge_keep: true # Keep worktree, window, and branch after merge by default
agent: claude

panes:
  - command: <agent> # Start the configured agent (e.g., claude)
    focus: true
  - split: horizontal # Second pane with default shell
```

### Project configuration example

`.muxix.yaml`:

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

For a full annotated example, see
[`docs/reference/example-config.yaml`](docs/reference/example-config.yaml).

### Configuration options

Most options have sensible defaults. You only need to configure what you want to
customize.

#### Basic options

| Option           | Description                                                                                           | Default                     |
| ---------------- | ----------------------------------------------------------------------------------------------------- | --------------------------- |
| `main_branch`    | Branch to merge into                                                                                  | Auto-detected               |
| `base_branch`    | Default base branch for new worktrees                                                                 | Current branch              |
| `worktree_dir`   | Directory for worktrees (absolute or relative). Supports `~` and `{project}`.                         | `<project>__worktrees/`     |
| `window_prefix`  | Prefix for tmux window/session names                                                                  | `wm-`                       |
| `mode`           | Tmux mode (`window` or `session`)                                                                     | `window`                    |
| `agent`          | Default agent for `<agent>` placeholder                                                               | `claude`                    |
| `agents`         | Named agent commands ([docs](https://muxix.dev/guide/agents#named-agents), global-only)       | `{}`                        |
| `merge_strategy` | Default merge strategy (`merge`, `rebase`, `squash`)                                                  | `merge`                     |
| `merge_keep`     | Keep resources after `muxix merge` by default                                                       | `false`                     |
| `theme`          | Dashboard color scheme ([custom colors](https://muxix.dev/guide/configuration#custom-colors)) | `default` (auto dark/light) |

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

Define your tmux pane layout with the `panes` array. For multiple windows in
session mode, use [`windows`](#multiple-windows-per-session) instead (they are
mutually exclusive).

```yaml
panes:
  - command: <agent>
    focus: true
  - command: npm run dev
    split: horizontal
    size: 15
```

Each pane supports:

| Option       | Description                                                    | Default |
| ------------ | -------------------------------------------------------------- | ------- |
| `command`    | Command to run (see [agent placeholders](#agent-placeholders)) | Shell   |
| `focus`      | Whether this pane receives focus                               | `false` |
| `zoom`       | Zoom pane to fullscreen (implies `focus: true`)                | `false` |
| `split`      | Split direction (`horizontal` or `vertical`)                   | —       |
| `size`       | Absolute size in lines/cells                                   | 50%     |
| `percentage` | Size as percentage (1-100)                                     | 50%     |

##### Agent placeholders

- `<agent>`: resolves to the configured agent (from `agent` config or `--agent`
  flag)

Built-in agents (`claude`, `gemini`, `codex`, `opencode`, `kiro-cli`, `vibe`,
`pi`) are auto-detected when used as literal commands and receive prompt
injection automatically, without needing the `<agent>` placeholder or a matching
`agent` config:

```yaml
panes:
  - command: 'claude --dangerously-skip-permissions'
    focus: true
  - command: 'codex --yolo'
    split: vertical
```

Each agent receives the prompt (via `-p`/`-P`/`-e`) using the correct format for
that agent. Auto-detection matches the executable name regardless of flags or
path.

#### Named layouts

Define reusable pane arrangements in the `layouts` map and select one at
add-time with `-l/--layout`:

```yaml
layouts:
  design:
    panes:
      - command: <agent>
        focus: true
      - command: <agent:codex>
        split: vertical
  review:
    panes:
      - command: <agent>
```

```bash
muxix add my-feature -l design
```

When `-l` is used, the layout's `panes` replace the top-level `panes` for that
worktree. All other config (hooks, files, agent, etc.) comes from the top-level
as usual. The `-l` flag cannot be combined with `--agent`.

#### File operations

New worktrees are clean checkouts with no gitignored files (`.env`,
`node_modules`, etc.). Use `files` to automatically copy or symlink what each
worktree needs:

```yaml
files:
  copy:
    - .env
  symlink:
    - .next/cache # Share build cache across worktrees
```

Both `copy` and `symlink` accept glob patterns.

To re-apply file operations to an existing worktree (e.g., after updating the
config), run `muxix sync-files` from inside the worktree. Use `--all` to sync
all worktrees at once.

#### Lifecycle hooks

Run commands at specific points in the worktree lifecycle, such as installing
dependencies or running database migrations. All hooks run with the **worktree
directory** as the working directory (or the nested config directory for
[nested configs](https://muxix.dev/guide/monorepos#nested-configuration))
and receive environment variables: `WM_HANDLE`, `WM_WORKTREE_PATH`,
`WM_PROJECT_ROOT`, `WM_CONFIG_DIR`.

`WM_CONFIG_DIR` points to the directory containing the `.muxix.yaml` that was
used, which may differ from `WM_WORKTREE_PATH` when using nested configs.

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

Agents in "working" status that produce no pane output for 10 seconds are
automatically detected as interrupted.

Set `status_format: false` to disable automatic tmux format modification

#### Default behavior

- Worktrees are created in `<project>__worktrees` as a sibling directory to your
  project by default
- If no `panes` configuration is defined, muxix provides opinionated defaults:
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

Here's how muxix organizes your worktrees by default:

```
~/projects/
├── my-project/               <-- Main project directory
│   ├── src/
│   ├── package.json
│   └── .muxix.yaml
│
└── my-project__worktrees/    <-- Worktrees created by muxix
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
The value supports `~` for the home directory and a `{project}` placeholder
that resolves to the main worktree's directory name. This lets a single
global config namespace every repo's worktrees under one root, e.g.
`worktree_dir: ~/.muxix/{project}`.

### Shell alias (recommended)

For faster typing, alias `muxix` to `wm`:

```bash
alias wm='muxix'
```

## Commands

- [`add`](#muxix-add-branch-name) - Create a new worktree and tmux window
- [`merge`](#muxix-merge-branch-name) - Merge a branch and clean up everything
- [`remove`](#muxix-remove-name-alias-rm) - Remove worktrees without merging
- [`list`](#muxix-list-alias-ls) - List all worktrees with status
- [`open`](#muxix-open-name) - Open a tmux window for an existing worktree
- [`close`](#muxix-close-name) - Close a worktree's tmux window (keeps
  worktree)
- [`resurrect`](#muxix-resurrect) - Restore worktree windows after a crash
- [`project`](#muxix-project) - Track project directories for `muxix start`
- [`start`](#muxix-start) - Launch all tracked projects (session per project,
  window per worktree)
- [`path`](#muxix-path-name) - Get the filesystem path of a worktree
- [`dashboard`](#muxix-dashboard) - Show TUI dashboard of all active agents
- [`sidebar`](#muxix-sidebar) - Toggle a compact agent status sidebar in tmux
- [`config edit`](#muxix-config-edit) - Edit the global configuration file
- [`init`](#muxix-init) - Generate configuration file
- [`sandbox`](#muxix-sandbox) - Manage sandbox backends (container/Lima)
- [`claude prune`](#muxix-claude-prune) - Clean up stale Claude Code entries
- [`completions`](#muxix-completions-shell) - Generate shell completions
- [`docs`](#muxix-docs) - Show detailed documentation

### `muxix add <branch-name>`

Creates a new git worktree with a matching tmux window and switches you to it
immediately. If the branch doesn't exist, it will be created automatically.

- `<branch-name>`: Name of the branch to create or switch to, a remote branch
  reference (e.g., `origin/feature-branch`), or a GitHub fork reference (e.g.,
  `user:branch`). Remote and fork references are automatically fetched and
  create a local branch with the derived name. Fork references derive the local
  branch as `user-branch` (e.g., `someuser:feature` creates local branch
  `someuser-feature`). Optional when using `--pr`.

#### Options

- `--base <branch|commit|tag>`: Specify a base branch, commit, or tag to branch
  from when creating a new branch. Overrides `base_branch` config. Defaults to
  `base_branch` from config, then the currently checked out branch.
- `--pr <number>`: Checkout a GitHub pull request by its number into a new
  worktree.
  - Requires the `gh` command-line tool to be installed and authenticated.
  - The local branch name defaults to the PR's head branch name, but can be
    overridden (e.g., `muxix add custom-name --pr 123`).
  - If that local branch already exists and has no worktree, it is reused.
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
- `--prompt-file-only`: Write the prompt file to the worktree without injecting
  it into agent commands. No agent pane is required. Useful when your editor has
  an embedded agent that reads `.muxix/PROMPT-*.md` directly.
- `-l, --layout <name>`: Use a named pane layout from config instead of the
  default panes. Cannot be combined with `--agent`.
- `-a, --agent <name>`: The agent(s) to use for the worktree(s). Can be
  specified multiple times to generate a worktree for each agent. Overrides the
  `agent` from your config file.
- `-W, --wait`: Block until the created tmux window is closed. Useful for
  scripting when you want to wait for an agent to complete its work. The agent
  can signal completion by running `muxix remove --keep-branch`.
- `-o, --open-if-exists`: If a worktree for the branch already exists, open it
  instead of failing. Similar to `tmux new-session -A`. Useful when you don't
  know or care whether the worktree already exists.
- `-s, --session`: Create a tmux session instead of a window. See
  [Session mode](#session-mode) for details.
- `--config <path>`: Use an alternate config file for this invocation. Still
  merges with global config.
- `--fork`: Fork the last conversation from the current worktree into the new
  one. The agent resumes with the forked conversation context. Use
  `--fork=<session-id>` to fork a specific session (prefix matching supported).
  Currently supports Claude Code.

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
muxix add user-auth

# Use an existing branch
muxix add existing-work

# Create a new branch from a specific base
muxix add hotfix --base production

# Create a worktree from a remote branch (creates local branch "user-auth-pr")
muxix add origin/user-auth-pr

# Remote branches with slashes work too (creates local branch "feature/foo")
muxix add origin/feature/foo

# Create a worktree in the background without switching to it
muxix add feature/parallel-task --background

# Use a custom name for the worktree directory and tmux window
muxix add feature/long-descriptive-branch-name --name short

# Open existing worktree if it exists, create if it doesn't (idempotent)
muxix add my-feature -o
```

##### Checking out pull requests and fork branches

```bash
# Checkout PR #123. The local branch will be named after the PR's branch.
muxix add --pr 123

# Checkout PR #456 with a custom local branch name
muxix add fix/api-bug --pr 456

# Checkout a fork branch using GitHub's owner:branch format (copy from GitHub UI)
# Creates local branch "someuser-feature-branch" tracking the fork
muxix add someuser:feature-branch
```

##### Moving changes to a new worktree

```bash
# Move uncommitted changes to a new worktree (including untracked files)
muxix add feature/new-thing --with-changes -u

# Move only staged/modified files (not untracked files)
muxix add fix/bug --with-changes

# Interactively select which changes to move
muxix add feature/partial --with-changes --patch
```

##### AI agent prompts

```bash
# Create a worktree with an inline prompt for AI agents
muxix add feature/ai --prompt "Implement user authentication with OAuth"

# Override the default agent for a specific worktree
muxix add feature/testing -a gemini

# Create a worktree with a prompt from a file
muxix add feature/refactor --prompt-file task-description.md

# Open your editor to write a prompt interactively
muxix add feature/new-api --prompt-editor

# Write prompt file only (for editors with embedded agents like neovim)
muxix add feature/task -P task.md --prompt-file-only
```

##### Skipping setup steps

```bash
# Skip expensive setup for documentation-only changes
muxix add docs-update --no-hooks --no-file-ops --no-pane-cmds

# Skip just the file operations (e.g., you don't need node_modules)
muxix add quick-fix --no-file-ops
```

##### Scripting with --wait

```bash
# Block until the agent completes and closes the window
muxix add feature/api --wait -p "Implement the REST API, then run: muxix remove --keep-branch"

# Use in a script to run sequential agent tasks
for task in task1.md task2.md task3.md; do
  muxix add "task-$(basename $task .md)" --wait -P "$task"
done
```

#### AI agent integration

When you provide a prompt via `--prompt`, `--prompt-file`, or `--prompt-editor`,
muxix automatically injects the prompt into panes running the configured agent
command (e.g., `claude`, `codex`, `opencode`, `gemini`, `kiro-cli`, `vibe`,
`pi`, or whatever you've set via the `agent` config or `--agent` flag) without
requiring any `.muxix.yaml` changes:

- Panes with a command matching the configured agent are automatically started
  with the given prompt.
- You can keep your `.muxix.yaml` pane configuration simple (e.g.,
  `panes: [{ command: "<agent>" }]`) and let muxix handle prompt injection at
  runtime.

This means you can launch AI agents with task-specific prompts without modifying
your project configuration for each task.

If your editor has an embedded agent (e.g., neovim with an agent plugin), use
`--prompt-file-only` to write the prompt to `.muxix/PROMPT-<branch>.md`
without requiring an agent pane. Your editor can then detect and consume the
file on startup. This can also be set permanently in config with
`prompt_file_only: true`.

#### Automatic branch name generation

The `--auto-name` (`-A`) flag generates a branch name from your prompt using an
LLM. The tool used depends on your configuration:

1. `auto_name.command` is set: uses that command as-is
2. `config.agent` is a known agent (`claude`, `gemini`, `codex`, `opencode`,
   `kiro-cli`, `vibe`, `pi`): uses the agent's CLI with a fast/cheap model
3. Neither: falls back to the [`llm`](https://llm.datasette.io/) CLI tool

##### Usage

```bash
# Opens editor for prompt, generates branch name
muxix add -A

# With inline prompt
muxix add -A -p "Add OAuth authentication"

# With prompt file
muxix add -A -P task-spec.md
```

##### Requirements

When `agent` is configured (e.g., `agent: claude`), muxix automatically uses
that agent's CLI for branch naming. No additional setup is required beyond
having the agent installed.

If no agent is configured and no `auto_name.command` is set, muxix uses the
`llm` CLI tool:

```bash
pipx install llm
```

Configure a model (e.g., OpenAI):

```bash
llm keys set openai
# Or use a local model
llm install llm-ollama
```

If you set `auto_name.command`, `llm` is not required.

##### Agent profile defaults

When an agent is configured, these commands are used automatically:

| Agent      | Auto-name command                                                        |
| ---------- | ------------------------------------------------------------------------ |
| `claude`   | `claude --model haiku -p`                                                |
| `gemini`   | `gemini -m gemini-2.5-flash-lite -p`                                     |
| `codex`    | `codex exec --config model_reasoning_effort="low" -m gpt-5.1-codex-mini` |
| `opencode` | `opencode run`                                                           |
| `kiro-cli` | `kiro-cli chat --no-interactive`                                         |
| `pi`       | `pi -p`                                                                  |

To override back to `llm` when an agent is configured, set
`auto_name.command: "llm"`.

##### Configuration

Optionally configure auto-name behavior in `.muxix.yaml`:

```yaml
auto_name:
  model: 'gemini-2.5-flash-lite'
  background: true # Always run in background when using --auto-name
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

To use a specific tool, set `auto_name.command`. The command string is split
into program and arguments, and the composed prompt is piped via stdin.

```yaml
auto_name:
  command: 'claude -p'

# Force llm even when an agent is configured
auto_name:
  command: 'llm'
```

| Option          | Description                                                      | Default                    |
| --------------- | ---------------------------------------------------------------- | -------------------------- |
| `command`       | Command for branch name generation (overrides agent profile)     | Agent profile or `llm` CLI |
| `model`         | LLM model to use with the `llm` CLI (ignored when `command` set) | `llm`'s default            |
| `background`    | Always run in background when using `--auto-name`                | `false`                    |
| `system_prompt` | Custom system prompt for branch name generation                  | Built-in prompt            |

Recommended models for fast, cheap branch name generation (with `llm`):

- `gemini-2.5-flash-lite` (recommended)
- `gpt-5-nano`

#### Parallel workflows & multi-worktree generation

muxix can generate multiple worktrees from a single `add` command, which is
ideal for running parallel experiments or delegating tasks to multiple AI
agents. This is controlled by four mutually exclusive modes:

- (`-a`, `--agent`): Create a worktree for each specified agent.
- (`-n`, `--count`): Create a specific number of worktrees.
- (`--foreach`): Create worktrees based on a matrix of variables.
- **stdin**: Pipe input lines to create worktrees with templated prompts.

When using any of these modes, branch names are generated from a template, and
prompts are templated with variables. Single-worktree prompts are passed through
literally, so common syntax like GitHub Actions `${{ ... }}` does not need to be
escaped.

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
  When set, muxix creates up to `<number>` worktrees, then waits for any
  window to close before starting the next. Requires agents to close windows
  when done (e.g., via prompt instruction to run
  `muxix remove --keep-branch`).

##### Prompt templating

When generating multiple worktrees, any prompt provided via `-p`, `-P`, or `-e`
is treated as a MiniJinja template. You can use variables from your generation
mode to create unique prompts for each agent or instance. For ordinary
single-worktree `add` commands, prompt text is not templated.

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
muxix add mobile-app --prompt-file mobile-task.md
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
muxix add refactor --prompt-file agent-task.md
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

You can pipe input lines to `muxix add` to create multiple worktrees. Each
line becomes available as the `{{ input }}` template variable in your prompt.
This is useful for batch-processing tasks from external sources.

**Plain text:** Each line becomes `{{ input }}`

```bash
echo -e "api\nauth\ndatabase" | muxix add refactor -P task.md
# {{ input }} = "api", "auth", "database"
```

**JSON lines:** Each key becomes a template variable

```bash
gh repo list --json url,name --jq -c '.[]' | muxix add analyze \
  --branch-template '{{ base_name }}-{{ name }}' \
  -P prompt.md
# Line: {"url":"https://github.com/lcensies/muxix","name":"muxix"}
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
muxix add my-feature -a claude -a gemini -p "Implement the new search API integration"
# Generates worktrees: my-feature-claude, my-feature-gemini

# Create 2 instances of the default agent
muxix add my-feature -n 2 -p "Implement task #{{ num }} in TASKS.md"
# Generates worktrees: my-feature-1, my-feature-2

# Create worktrees from a variable matrix
muxix add my-feature --foreach "platform:iOS,Android" -p "Build for {{ platform }}"
# Generates worktrees: my-feature-ios, my-feature-android

# Create agent-specific worktrees via --foreach
muxix add my-feature --foreach "agent:claude,gemini" -p "Implement the dashboard refactor"
# Generates worktrees: my-feature-claude, my-feature-gemini

# Use frontmatter in a prompt file for cleaner syntax
# task.md contains:
# ---
# foreach:
#   env: [staging, production]
#   task: [smoke-tests, integration-tests]
# ---
# Run {{ task }} against the {{ env }} environment
muxix add testing --prompt-file task.md
# Generates worktrees: testing-staging-smoke-tests, testing-production-integration-tests

# Pipe input from stdin to create worktrees
# review.md contains: Review the {{ input }} module for security issues.
echo -e "auth\npayments\napi" | muxix add review -A -P review.md
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
# When done, run: muxix remove --keep-branch

find src/utils -name "*.ts" ! -name "*.test.ts" | \
  muxix add add-tests \
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

### `muxix merge [branch-name]`

Merges a branch into a target branch (main by default) and automatically cleans
up all associated resources (worktree, tmux window, and local branch).

<!-- prettier-ignore -->
> [!TIP]
> **`merge` vs `remove`**: Use `merge` when you want to merge directly
> without a pull request. If your workflow uses pull requests, use
> [`remove`](#muxix-remove-name-alias-rm) to clean up after your PR is merged
> on the remote.

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
- `--cleanup`: Clean up after merging, overriding `merge_keep: true`.
- `--notification`: Show a system notification on successful merge. Useful when
  delegating merge to an AI agent and you want to be notified when it completes.

#### Merge strategies

By default, `muxix merge` performs a standard merge commit (configurable via
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
# ~/.config/muxix/config.yaml
merge_strategy: rebase
```

To keep the worktree, window, and branch after every merge unless overridden,
set:

```yaml
merge_keep: true
```

Use `muxix merge --cleanup` to clean up for a single merge when this default is
enabled.

#### What happens

1. Determines which branch to merge (specified branch or current branch if
   omitted)
2. Determines the target branch (`--into` or main branch from config)
3. Checks for uncommitted changes (errors if found, unless
   `--ignore-uncommitted` is used)
4. Commits staged changes if present (unless `--ignore-uncommitted` is used)
5. Merges your branch into the target using the selected strategy (default:
   merge commit)
6. Deletes the tmux window unless keep behavior is enabled via `--keep` or
   `merge_keep: true`
7. Removes the worktree unless keep behavior is enabled via `--keep` or
   `merge_keep: true`
8. Deletes the local branch unless keep behavior is enabled via `--keep` or
   `merge_keep: true`

#### Typical workflow

Run `muxix merge` from inside the worktree's tmux window: it detects the branch
you are on, merges it into main, and closes the window as part of cleanup.

#### Examples

```bash
# Merge branch into main (default: merge commit)
muxix merge user-auth

# Merge the current worktree you're in
# (run this from within the worktree's tmux window)
muxix merge

# Rebase onto main before merging for a linear history
muxix merge user-auth --rebase

# Squash all commits into a single commit
muxix merge user-auth --squash

# Merge but keep the worktree/window/branch to verify before cleanup
muxix merge user-auth --keep
# ... verify the merge in main ...
muxix remove user-auth  # clean up later when ready

# Merge into a different branch (stacked PRs)
muxix merge feature/subtask --into feature/parent
```

---

### `muxix remove [name]...` (alias: `rm`)

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
muxix remove

# Remove a specific worktree with confirmation if unmerged
muxix remove experiment

# Remove multiple worktrees at once
muxix rm feature-a feature-b feature-c

# Remove multiple worktrees with force (no confirmation)
muxix rm -f old-work stale-branch

# Use the alias
muxix rm old-work

# Remove worktree/window but keep the branch
muxix remove --keep-branch experiment

# Force remove without prompts
muxix rm -f experiment

# Remove worktrees whose remote branches were deleted (e.g., after PR merge)
muxix rm --gone

# Force remove all gone worktrees (no confirmation)
muxix rm --gone -f

# Remove all worktrees at once
muxix rm --all
```

---

### `muxix rename [old-name] <new-name>`

Renames a worktree's directory, its tmux window or session, and the per-worktree
muxix metadata. Optionally also renames the underlying git branch.

- `[old-name]`: Optional current worktree name. Defaults to the current worktree
  when run from inside one.
- `<new-name>`: The new handle (directory name and tmux window/session base name).

#### Options

- `--branch`, `-b`: Also rename the underlying git branch to match `<new-name>`.
  Fails if the worktree is on a detached HEAD.

#### Examples

```bash
# Rename a worktree from inside it
muxix rename feature-new

# Rename a specific worktree by name
muxix rename feature-old feature-new

# Also rename the branch to match
muxix rename feature-old feature-new --branch
```

Rename is non-destructive: uncommitted changes and untracked files are
preserved. The main worktree cannot be renamed. Collisions (existing target
path, existing tmux target, or existing branch) are rejected before any changes
are made.

---

### `muxix list` (alias: `ls`)

Lists all git worktrees with their agent status, multiplexer window status, and
merge status. Supports filtering by worktree handle or branch name.

#### Arguments

- `[worktree-or-branch...]`: Filter by worktree handle (directory name) or
  branch name. Accepts multiple values. When omitted, shows all worktrees.

#### Options

- `--pr`: Show GitHub PR status for each worktree. Requires the `gh` CLI to be
  installed and authenticated. Note that it shows pull requests' statuses with
  [Nerd Font](https://www.nerdfonts.com/) icons, which requires Nerd Font
  compatible font installed.
- `--json`: Output as JSON. Produces a JSON array of objects with fields:
  `handle`, `branch`, `path`, `is_main`, `mode`, `has_uncommitted_changes`,
  `is_open`, `created_at`.

#### Examples

```bash
# List all worktrees
muxix list

# List with PR status
muxix list --pr

# Output as JSON for scripting
muxix list --json

# Filter to specific worktrees
muxix list my-feature
muxix list feature-auth feature-api
```

#### Example output

```
BRANCH      AGE  AGENT  MUX  UNMERGED  PATH
main        -    -      -    -         ~/project
user-auth   2h   🤖     ✓    -         ~/project__worktrees/user-auth
bug-fix     3d   ✅     ✓    ●         ~/project__worktrees/bug-fix
api-work    1w   -      ✓    -         ~/project__worktrees/api-work
```

#### Key

- AGE shows how old the worktree is (e.g., `2h`, `3d`, `1w`, `2mo`)
- AGENT shows the current agent status (see
  [status tracking](https://muxix.dev/guide/status-tracking/)):
  - `🤖` = working, `💬` = waiting for input, `✅` = finished
  - Multiple agents per worktree show a count (e.g., `2🤖 1✅`)
- `✓` in MUX column = multiplexer window exists for this worktree
- `●` in UNMERGED column = branch has commits not merged into main
- `-` = not applicable

---

### `muxix config edit`

Opens the global configuration file (`~/.config/muxix/config.yaml`) in your
preferred editor. Uses `$VISUAL`, `$EDITOR`, or falls back to `vi`. Creates the
file with commented-out defaults if it doesn't exist yet.

---

### `muxix config path`

Prints the path to the global configuration file. Useful for scripting.

---

### `muxix config reference`

Prints the default configuration file with all options documented. Useful for
discovering available options or piping to an AI agent for context.

---

### `muxix init`

Generates `.muxix.yaml` with example configuration and `"<global>"`
placeholder usage.

---

### `muxix open [name...]`

Opens or switches to a tmux window for a pre-existing git worktree. If the
window already exists, switches to it. If not, creates a new window with the
configured pane layout and environment. Accepts multiple names to open several
worktrees at once.

- `[name...]`: One or more worktree names (the directory name, which is also the
  tmux window name without the prefix). Optional with `--new` when run from
  inside a worktree.

#### Options

- `-n, --new`: Force opening in a new window even if one already exists. Creates
  a duplicate window with a suffix (e.g., `-2`, `-3`). Useful for having
  multiple terminal views into the same worktree.
- `-s, --session`: Open in session mode, overriding the stored mode. Persists
  the mode change for subsequent opens. Cannot be combined with `--new`. Only
  supported with tmux.
- `--config <path>`: Use an alternate config file for this invocation. Still
  merges with global config.
- `--run-hooks`: Re-runs the `post_create` commands (these block window
  creation).
- `--force-files`: Re-applies file copy/symlink operations. Useful for restoring
  a deleted `.env` file.
- `-p, --prompt <text>`: Provide an inline prompt for AI agent panes.
- `-P, --prompt-file <path>`: Provide a path to a file containing the prompt.
- `-c, --continue`: Resume the agent's most recent conversation in this
  worktree. Injects the appropriate flag for the configured agent (e.g.,
  `--continue` for Claude, `--resume` for Gemini).
- `-e, --prompt-editor`: Open your editor to write the prompt interactively.
- `--prompt-file-only`: Write the prompt file without injecting it into agent
  commands.

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
muxix open user-auth

# Force open a second window for the same worktree (creates user-auth-2)
muxix open user-auth --new

# Open a new window for the current worktree (run from within the worktree)
muxix open --new

# Open in session mode (converts from window mode if needed)
muxix open user-auth --session

# Resume the agent's last conversation
muxix open user-auth --continue

# Resume and send a follow-up prompt
muxix open user-auth --continue -p "Continue implementing the login flow"

# Open and re-run dependency installation
muxix open user-auth --run-hooks

# Open and restore configuration files
muxix open user-auth --force-files

# Open multiple worktrees at once
muxix open user-auth api-refactor bugfix-login
```

---

### `muxix close [name]`

Closes the tmux window for a worktree without removing the worktree or branch.
This is useful when you want to temporarily close a window to reduce clutter or
free resources, but plan to return to the work later.

- `[name]`: Optional worktree name (the directory name). Defaults to current
  directory if omitted.

#### Examples

```bash
# Close the window for a specific worktree
muxix close user-auth

# Close the current worktree's window (run from within the worktree)
muxix close
```

To reopen the window later, use [`muxix open`](#muxix-open-name).

**Tip**: You can also use tmux's native kill-window command (default:
`prefix + &`) to close a worktree's window with the same effect.

---

### `muxix resurrect`

Restores worktree windows after a tmux or computer crash. Uses persisted agent
state files to detect which worktrees had active agents before the crash, then
reopens them with `--continue` to resume agent conversations.

#### Options

- `--dry-run`: Show what would be restored without doing it.

#### Examples

```bash
# See what would be restored after a crash
muxix resurrect --dry-run

# Restore all worktrees that had agents running
muxix resurrect
```

#### How it works

1. Reads agent state files from `~/.local/state/muxix/agents/`
2. Matches each state file's working directory to a git worktree in the current
   repo
3. Skips worktrees that are already open or no longer exist
4. Opens each matched worktree with `--continue` to resume the agent

---

### `muxix project`

Manages the list of tracked project directories used by `muxix start`. The
registry is a plain YAML list at `~/.config/muxix/projects.yaml`.

```bash
muxix project add ~/repos/my-app   # track a project
muxix project list                 # list tracked projects
muxix project rm my-app            # untrack by name or path
muxix project open my-app          # start that project's session and focus it
```

`project open <name|path>` does what `muxix start` does for a single
project (session + base layout + worktree windows, idempotent), then focuses
the session — switching the client inside tmux, attaching from a plain shell.
It accepts the same `-c`/`--continue` agent resume flag.

As a shortcut, `muxix add <dir>` tracks the directory as a project when the
argument is an existing directory containing `.git`.

---

### `muxix start`

Launches every tracked project: one tmux session per project (base layout from
`.muxix.yaml` `windows:`, `~/.config/tmuxrs/<name>.yml`, or
`~/.config/tmuxinator/<name>.yml`), plus one window per muxix worktree.
Idempotent — existing sessions and windows are left untouched.

#### Options

- `-c`, `--continue`: Relaunch the last coding agent in each project/worktree,
  resuming its previous conversation where possible (same resume ladder as
  `muxix resurrect`).

```bash
muxix start        # open all tracked projects
muxix start -c     # ...and resume the last agent everywhere
```

See the [Projects guide](https://muxix.dev/guide/projects) for details.

---

### `muxix sync-files`

Re-applies file operations (copy and symlink from `files` config) to existing
worktrees. Useful when you add new entries to the `files` config or a symlink
was accidentally deleted.

#### Options

- `--all`: Sync all worktrees instead of just the current one.

#### Examples

```bash
# Sync files to the current worktree
muxix sync-files

# Sync files to all worktrees
muxix sync-files --all
```

---

### `muxix path <name>`

Prints the filesystem path of an existing worktree. Useful for scripting or
quickly navigating to a worktree directory.

- `<name>`: Worktree name (the directory name).

#### Examples

```bash
# Get the path of a worktree
muxix path user-auth
# Output: /Users/you/project__worktrees/user-auth

# Use in scripts or with cd
cd "$(muxix path user-auth)"

# Copy a file to a worktree
cp config.json "$(muxix path feature-branch)/"
```

---

### `muxix dashboard`

Opens a TUI dashboard showing all active AI agents across all tmux sessions.
Useful for monitoring multiple parallel agents and quickly jumping between them.

#### Options

- `-d, --diff`: Open the diff view directly for the current worktree. Useful
  when you want to quickly review uncommitted changes without navigating through
  the agent list.
- `-P, --preview-size <10-90>`: Set preview pane size as percentage (larger =
  more preview, less table). Default: 60.
- `-s, --session`: Filter to only show agents in the current session. Useful for
  session-per-project workflows where each session maps to a different
  repository.
- `-t, --tab <agents|worktrees>`: Open directly on the specified tab.

<!-- prettier-ignore -->
> [!IMPORTANT]
> This feature requires [agent status tracking](#agent-status-tracking) to be
> configured. Without it, no agents will appear in the dashboard.

![muxix dashboard](https://raw.githubusercontent.com/lcensies/muxix/refs/heads/main/meta/dashboard.webp)

#### Keybindings

| Key       | Action                                  |
| --------- | --------------------------------------- |
| `1`-`9`   | Quick jump to agent (closes dashboard)  |
| `Tab`     | Toggle between current and last agent   |
| `d`       | View diff (opens WIP view)              |
| `o`       | Open PR in browser                      |
| `p`       | Peek at agent (dashboard stays open)    |
| `s`       | Cycle sort mode                         |
| `/`       | Filter agents by name                   |
| `F`       | Toggle session filter                   |
| `f`       | Toggle stale filter (show/hide stale)   |
| `i`       | Enter input mode (type to agent)        |
| `Ctrl+u`  | Scroll preview up                       |
| `Ctrl+d`  | Scroll preview down                     |
| `+`/`-`   | Resize preview pane                     |
| `Enter`   | Go to selected agent (closes dashboard) |
| `j`/`k`   | Navigate up/down                        |
| `:`       | Open command palette                    |
| `q`/`Esc` | Quit                                    |

#### Live preview

The bottom half shows a live preview of the selected agent's terminal output.
The preview auto-scrolls to show the latest output, but you can scroll through
history with `Ctrl+u`/`Ctrl+d`. Press `i` to enter input mode and type directly
to the agent without leaving the dashboard.

#### Columns

- **#**: Quick jump key (1-9)
- **Project**: Project name (from `__worktrees` path or directory name)
- **Agent**: Worktree/window name
- **Git**: Diff stats showing branch changes (dim) and uncommitted changes
  (bright). Shows a rebase icon when a rebase is in progress.
- **Status**: Agent status icon (🤖 working, 💬 waiting, ✅ done, or "stale")
- **Time**: Time since last status change
- **Title**: Claude Code session title (auto-generated summary)

#### Sort modes

Press `s` to cycle through sort modes:

- **Priority** (default): Waiting > Done > Working > Stale
- **Project**: Group by project name, then by priority within each project
- **Recency**: Most recently updated first
- **Natural**: Original tmux order (by pane creation)

Your sort preference persists in the tmux session.

#### Session filter

Press `F` to toggle the session filter. When active, only agents in the current
session are shown. This is useful for session-per-project workflows where each
session maps to a repository. You can also start the dashboard with `--session`
to default to session filtering. The preference persists across sessions.

#### Name filter

Press `/` to activate the name filter. Type to filter the agent list by project
or worktree name (case-insensitive). Press `Enter` to accept the filter and
return to normal navigation, or `Esc` to clear the filter. When a filter is
active, it is shown in the footer bar.

#### Stale filter

Press `f` to toggle between showing all agents or hiding stale ones. The filter
state persists across dashboard sessions within the same tmux server.

#### Diff view

Press `d` to view the diff for the selected agent. The diff view has two modes:

- **WIP** - Shows uncommitted changes (`git diff HEAD`)
- **review** - Shows all changes on the branch vs main (`git diff main...HEAD`)

Press `Tab` to toggle between modes. The footer displays which mode is active
along with diff statistics showing lines added (+) and removed (-).

| Key       | Action                           |
| --------- | -------------------------------- |
| `Tab`     | Toggle WIP / review              |
| `a`       | Enter patch mode (WIP only)      |
| `j`/`k`   | Scroll down/up                   |
| `Ctrl+d`  | Page down                        |
| `Ctrl+u`  | Page up                          |
| `c`       | Send commit command to agent     |
| `m`       | Trigger merge and exit dashboard |
| `:`       | Open command palette             |
| `q`/`Esc` | Close diff view                  |

#### Patch mode

Patch mode (`a` from WIP diff) allows staging individual hunks like
`git add -p`. This is useful for selectively staging parts of an agent's work.

When [delta](https://github.com/dandavison/delta) is installed, hunks are
rendered with syntax highlighting for better readability.

| Key       | Action                           |
| --------- | -------------------------------- |
| `y`       | Stage current hunk               |
| `n`       | Skip current hunk                |
| `u`       | Undo last staged hunk            |
| `s`       | Split hunk (if splittable)       |
| `o`       | Comment on hunk (sends to agent) |
| `j`/`k`   | Navigate to next/previous hunk   |
| `:`       | Open command palette             |
| `q`/`Esc` | Exit patch mode                  |

Press `y` to stage the current hunk and advance to the next. Press `n` to skip
without staging. The counter in the header shows your progress (e.g., `[3/10]`).

Press `s` to split the current hunk into smaller pieces when there are context
lines between separate changes. Press `u` to undo the last staged hunk.

Press `o` to comment on the current hunk. This sends a message to the agent
including the file path, line number, the diff hunk as context, and your
comment. Useful for giving feedback like "This function should handle the error
case".

#### Example tmux binding

Add to your `~/.tmux.conf` for quick access:

```bash
bind C-s display-popup -h 30 -w 100 -E "muxix dashboard"

# Open directly on Worktrees tab
bind C-w display-popup -h 30 -w 100 -E "muxix dashboard --tab worktrees"
```

Then press `prefix + Ctrl-s` to open the dashboard as a tmux popup.

---

### `muxix sidebar`

Toggles a live agent status sidebar on the left side of all tmux windows. Shows
all active agents across all sessions and projects with live status updates,
providing an always-visible overview without taking over the full screen like
the dashboard.

```bash
muxix sidebar            # Toggle sidebar on/off (all sessions)
muxix sidebar --session  # Toggle current session only, or opt out of global mode
```

The sidebar displays:

- Status icon (working/waiting/done with spinner animation)
- Project and worktree name (e.g. `myproject/fix-bug`)
- Elapsed time since last status change

| Key     | Action             |
| ------- | ------------------ |
| `j`/`k` | Navigate up/down   |
| `Enter` | Jump to agent      |
| `g`/`G` | Jump to first/last |
| `v`     | Toggle layout mode |
| `q`     | Quit sidebar       |

When the global sidebar is active, `muxix sidebar --session` hides it in the
current tmux session only. Run the same command again to show it in that session
again while keeping the global sidebar active elsewhere.

Configure width and layout in `.muxix.yaml`:

```yaml
sidebar:
  width: 40 # absolute columns, or "15%" for percentage
  layout: tiles # "compact" or "tiles" (default)
```

#### Example tmux binding

```bash
bind C-t run-shell "muxix sidebar"
```

Then press `prefix + Ctrl-t` to toggle the sidebar.

> **Note:** The sidebar is currently tmux-only. When enabled, a sidebar pane is
> created in every existing window, and new windows automatically get one via a
> tmux hook.

---

### `muxix sandbox`

Commands for managing sandbox functionality. See the
[sandbox guide](https://muxix.dev/guide/sandbox/) for full
documentation.

| Command               | Description                                            |
| --------------------- | ------------------------------------------------------ |
| `sandbox pull`        | Pull the latest container image from the registry      |
| `sandbox build`       | Build the container image locally                      |
| `sandbox shell`       | Start an interactive shell inside a sandbox            |
| `sandbox agent`       | Run the configured agent in a sandbox with RPC support |
| `sandbox stop`        | Stop running Lima VMs                                  |
| `sandbox prune`       | Delete unused Lima VMs to reclaim disk space           |
| `sandbox install-dev` | Cross-compile and install muxix into sandboxes (dev) |

---

### `muxix claude prune`

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
muxix claude prune
```

#### Example output

```
  - Removing: /Users/user/project__worktrees/old-feature

✓ Created backup at ~/.claude.json.bak
✓ Removed 3 stale entries from ~/.claude.json
```

---

### `muxix completions <shell>`

Generates shell completion script for the specified shell. Completions provide
tab-completion for commands and dynamic branch name suggestions.

- `<shell>`: Shell type: `bash`, `zsh`, or `fish`.

#### Examples

```bash
# Generate completions for zsh
muxix completions zsh
```

See the [Shell Completions](#shell-completions) section for installation
instructions.

---

### `muxix docs`

Displays this README with terminal formatting. Useful for quick reference
without leaving the terminal.

When run interactively, renders markdown with colors and uses a pager (`less`).
When piped (e.g., to an LLM), outputs raw markdown for clean context.

#### Using with AI agents

You can ask an agent to read the docs and configure muxix for you:

```
> run `muxix docs` and configure muxix so that on the left pane
  there is claude as agent, and on the right side neovim and empty
  shell on top of each other

⏺ Bash(muxix docs)
  ⎿  <p align="center">
       <picture>
     … +923 lines

⏺ Write(.muxix.yaml)
  ⎿  Wrote 9 lines to .muxix.yaml

⏺ Created .muxix.yaml with the layout:
  - Left: claude agent (focused)
  - Right top: neovim
  - Right bottom: empty shell
```

## Declarative agent harness

One `bootstrap:` block provisions every agent CLI you have installed, and
`muxix setup` makes it true:

```yaml
bootstrap:
  skills:
    - path: ./skills/auto-git
  subagents:
    - path: ./agents/reviewer.md
  prompt_components: [fff]
  features:
    ponytail:
      pi: git:github.com/DietrichGebert/ponytail  # pi installs a plugin
      default: ponytail                           # everyone else gets the prompt
  agents:
    omp:
      settings:            # RFC 7386 merge patch, in the agent's own format
        modelRoles: { smol: zai/glm-5.1 }
```

Declarations are agent-agnostic; what lands where is the agent's own layout.
Anything an agent cannot take is reported as `skipped` **with the reason** —
never silently dropped.

| | claude | codex | copilot | gemini | opencode | pi | omp |
|---|---|---|---|---|---|---|---|
| skills | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| subagents | ✅ | — | ✅ | ✅ | ✅ | ✅ | ✅ |
| prompt components / features | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| plugins | ✅ | — | — | — | ✅ | ✅ | ✅ |
| hooks | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | — |
| MCP servers | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| `settings` patch | ✅ | — | ✅ | ✅ | ✅ | ✅ | ✅ |
| theme | ✅ | — | — | ✅ | ✅ | — | — |
| config profiles (`exec --profile`) | ✅ | ✅ | ✅ | — | — | ✅ | ✅ |

Each gap has a documented cause (Codex subagents are TOML config layers, its
settings are TOML, omp's hooks have no compat plugin, Gemini/OpenCode expose no
config-dir redirect), listed in
[the bootstrap guide](https://muxix.dev/guide/bootstrap).

Per-agent **config profiles** layer a named overlay on top of an agent's own
config dir — `muxix exec --profile corp pi` runs pi with a different provider,
plugin set and session history, without touching your base config. See
[profiles](https://muxix.dev/guide/profiles).

## Agent status tracking

Muxix can display the status of the agent in your tmux window list, giving you
at-a-glance visibility into what the agent in each window doing.

![tmux status showing agent icons](https://raw.githubusercontent.com/lcensies/muxix/refs/heads/main/meta/status.webp)

#### Key

- 🤖 = agent is working
- 💬 = agent is waiting for user input
- ✅ = agent finished (auto-clears on window focus)

| Agent        | Status                                                                      |
| ------------ | --------------------------------------------------------------------------- |
| Claude Code  | ✅ Supported                                                                |
| OpenCode     | ✅ Supported                                                                |
| Codex        | ✅ Supported\*                                                              |
| Copilot CLI  | ✅ Supported\*                                                              |
| Pi           | ✅ Supported\*                                                              |
| Gemini CLI   | ✅ Supported                                                                |
| omp          | ✅ Supported\*                                                              |
| Kiro         | [Tracking issue](https://github.com/kirodotdev/Kiro/issues/5440)            |
| Mistral Vibe | [Tracking issue](https://github.com/mistralai/mistral-vibe/discussions/334) |

**Notes:**

- **Codex**: No 💬 waiting state
- **Copilot CLI**: No 💬 waiting state
- **Pi**: No 💬 waiting state
- **omp**: No 💬 waiting state (pi-compatible extension)
- **Kiro**: Hooks support is messy: requires a custom agent since the default
  can't be edited

### Setup

Run `muxix setup` to automatically detect your agent CLIs, install status
tracking hooks, and install skills:

```bash
muxix setup
```

You can also run specific parts: `muxix setup --hooks` or
`muxix setup --skills`. For Claude Code, `CLAUDE_CONFIG_DIR` is respected for
both hook and skill installation.

Muxix will also prompt you on first run if it detects an agent without status
tracking or skills configured.

Muxix automatically modifies your tmux `window-status-format` to display the
status icons. This happens once per session and only affects the current tmux
session (not your global config).

#### Manual setup

If you prefer manual setup:

**Claude Code**: install the muxix status plugin:

```
claude plugin marketplace add lcensies/muxix
claude plugin install muxix-status
```

Or manually add the hooks to `~/.claude/settings.json`. See
[.claude-plugin/plugin.json](.claude-plugin/plugin.json) for the hook
configuration.

**Copilot CLI**: copy the hooks to your repository:

```bash
mkdir -p .github/hooks/muxix-status
curl -o .github/hooks/muxix-status/hooks.json \
  https://raw.githubusercontent.com/lcensies/muxix/main/.github/hooks/muxix-status/hooks.json
```

Note: Copilot hooks are per-repository. The waiting state is not supported due
to limitations in the Copilot CLI hooks implementation.

**OpenCode**: download the muxix status plugin:

```bash
mkdir -p ~/.config/opencode/plugins
curl -o ~/.config/opencode/package.json \
  https://raw.githubusercontent.com/lcensies/muxix/main/resources/opencode/package.json
curl -o ~/.config/opencode/plugins/muxix-status.ts \
  https://raw.githubusercontent.com/lcensies/muxix/main/resources/opencode/plugins/muxix-status.ts
```

Restart OpenCode for the plugin to take effect.

### Customization

You can customize the icons in your config:

```yaml
# ~/.config/muxix/config.yaml
status_icons:
  working: '🔄'
  waiting: '⏸️'
  done: '✔️'
```

If you prefer to manage the tmux format yourself, disable auto-modification and
add the status variable to your `~/.tmux.conf`:

```yaml
# ~/.config/muxix/config.yaml
status_format: false
```

```bash
# ~/.tmux.conf
set -g window-status-format '#I:#W#{?@muxix_status, #{@muxix_status},}#{?window_flags,#{window_flags}, }'
set -g window-status-current-format '#I:#W#{?@muxix_status, #{@muxix_status},}#{?window_flags,#{window_flags}, }'
```

### Jump to completed or waiting agents

Use `muxix last-done` to quickly switch to the agent that most recently
finished its task or is waiting for user input. Repeated invocations cycle
through all completed and waiting agents in reverse chronological order.

Add a tmux keybinding for quick access:

```bash
# ~/.tmux.conf
bind-key L run-shell "muxix last-done"
```

Then press `prefix + L` to jump to the last completed or waiting agent, press
again to cycle to the next oldest, and so on.

### Toggle between agents

Use `muxix last-agent` to toggle between your current agent and the last one
you visited. This works like vim's `Ctrl+^` or tmux's `last-window` - it
remembers which agent you came from and switches back to it. Pressing it again
returns you to where you were.

This is available both as a CLI command and as the `Tab` key in the dashboard.

Add a tmux keybinding for quick access:

```bash
# ~/.tmux.conf
bind Tab run-shell "muxix last-agent"
```

Then press `prefix + Tab` to toggle between your two most recent agents.

## Sandbox

muxix can run agents inside containers (Docker/Podman/Apple Container) or Lima
VMs, isolating them from your host. Agents are restricted to the project
worktree; sensitive files like SSH keys, AWS credentials, and other secrets are
not accessible. This lets you run agents with `--dangerously-skip-permissions`
without worrying about what they might touch on your host.

Sandboxing is transparent: status indicators, the dashboard, spawning new
agents, and merging all continue to work normally across the sandbox boundary.

### Backends

|                 | Container (Docker/Podman/Apple Container)  | Lima VM                         |
| --------------- | ------------------------------------------ | ------------------------------- |
| **Isolation**   | Process/VM-level                           | Machine-level (virtual machine) |
| **Persistence** | Ephemeral (new container per session)      | Persistent (stateful VMs)       |
| **Toolchain**   | Custom Dockerfile or host command proxying | Built-in Nix & Devbox support   |
| **Network**     | Optional restrictions (domain allowlist)   | Unrestricted                    |

Container is a good default: simple to set up and ephemeral, so no state
accumulates between sessions. Choose Lima if you want persistent VMs with
built-in Nix/Devbox toolchain support.

### Quick start

```yaml
# ~/.config/muxix/config.yaml or .muxix.yaml
sandbox:
  enabled: true
  # backend: lima  # uncomment for Lima VMs (default: container)
```

The pre-built container image is pulled automatically on first run. For Lima,
the VM is created and provisioned on first use.

### Shared features

Both backends support:

- **Host command proxying**: Run specific commands (build tools, linters) on the
  host from inside the sandbox via `host_commands` config
- **Extra mounts**: Mount additional host directories into the sandbox
  (read-only by default)
- **Git identity**: Your `user.name` and `user.email` are automatically injected
  so git commits work without exposing your full `~/.gitconfig`
- **Credential sharing**: Agent credentials are shared between host and sandbox
- **Network restrictions** (container only): Block outbound connections except
  to approved domains

See the [sandbox guide](https://muxix.dev/guide/sandbox/) for full
setup, configuration, and security details.

## Session mode

By default, muxix creates tmux **windows** within your current session. With
session mode, each worktree gets its own **tmux session** instead. This allows
each worktree to have multiple windows.

### Enabling session mode

Add to your config:

```yaml
# ~/.config/muxix/config.yaml or .muxix.yaml
mode: session
```

Or use the `--session` flag:

```bash
muxix add feature-branch --session
```

### How it works

- **Persistence**: The mode is stored per-worktree. If you create a worktree
  with `--session`, subsequent `open`/`close`/`remove` commands automatically
  use session mode for that worktree.
- **Navigation**: After `merge` or `remove`, muxix switches you back to the
  previous session.

### Multiple windows per session

Use the `windows` config to launch multiple windows in each session. Each window
can have its own pane layout. This is mutually exclusive with the top-level
`panes` config.

```yaml
mode: session
windows:
  - name: editor
    panes:
      - command: <agent>
        focus: true
      - split: horizontal
        size: 20
  - name: tests
    panes:
      - command: just test --watch
  - panes:
      - command: tail -f app.log
```

Each window supports:

| Option  | Description                                            | Default      |
| ------- | ------------------------------------------------------ | ------------ |
| `name`  | Window name (if omitted, tmux auto-names from command) | Auto         |
| `panes` | Pane layout (same syntax as top-level `panes`)         | Single shell |

`focus: true` works across windows: the last pane with focus set determines
which window is selected when the session opens.

### Limitations

- **tmux only**: Session mode is currently only supported for the tmux backend.
- **No duplicates**: Unlike window mode which supports opening multiple windows
  for the same worktree (`-2`, `-3` suffixes), session mode creates one session
  per worktree.


## Why git worktrees?

[Git worktrees](https://git-scm.com/docs/git-worktree) keep several branches
checked out at once, each in its own directory, so switching tasks is a `cd`
rather than a `git stash`, and builds, installs or tests in one branch never
disturb another. muxix pairs each worktree with a tmux window and automates
the setup and teardown around it.

## Git worktree caveats

Worktrees have nuances muxix automates around; the mechanics are worth knowing.

- [Gitignored files require configuration](#gitignored-files-require-configuration)
- [Conflicts](#conflicts)
- [Package manager considerations (pnpm, yarn)](#package-manager-considerations-pnpm-yarn)
- [Rust projects](#rust-projects)
- [Port conflicts in monorepos](#port-conflicts-in-monorepos)
- [Symlinks and `.gitignore` trailing slashes](#symlinks-and-gitignore-trailing-slashes)
- [Local git ignores (`.git/info/exclude`) are not shared](#local-git-ignores-gitinfoexclude-are-not-shared)

### Gitignored files require configuration

When `git worktree add` creates a new working directory, it's a clean checkout.
Files listed in your `.gitignore` (e.g., `.env` files, `node_modules`, IDE
configuration) will not exist in the new worktree by default. Your application
will be broken in the new worktree until you manually create or link these
necessary files.

This is a primary feature of muxix. Use the `files` section in your
`.muxix.yaml` to automatically copy or symlink these files on creation:

```yaml
# .muxix.yaml
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
take significant time. muxix has a
[special cleanup mechanism](https://github.com/lcensies/muxix/blob/main/src/scripts/cleanup_node_modules.sh)
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

### Port conflicts in monorepos

When running multiple services (API, web app, database) in a monorepo, each
worktree needs unique ports to avoid conflicts. For example, if your `.env` has
hardcoded ports like `API_PORT=3001` and `VITE_PORT=3000`, running two worktrees
simultaneously would fail because both would try to bind to the same ports.
Simply copying `.env` files won't work since all worktrees would use the same
ports.

**Solution**: Use a `post_create` hook to generate a `.env.local` file with
unique ports. Many frameworks (Vite, Next.js, CRA) automatically load
`.env.local` and merge it with `.env`, with `.env.local` taking precedence. For
plain Node.js, use multiple `--env-file` flags where later files override
earlier ones.

Create a script at `scripts/worktree-env`:

```bash
#!/usr/bin/env bash
set -euo pipefail

port_in_use() {
  lsof -nP -iTCP:"$1" -sTCP:LISTEN &>/dev/null
}

find_port() {
  local port=$1
  while port_in_use "$port"; do
    ((port++))
  done
  echo "$port"
}

# Hash the handle to get a deterministic port offset (0-99)
hash=$(echo -n "$WM_HANDLE" | md5 | cut -c1-4)
offset=$((16#$hash % 100))

# Find available ports starting from the hash-based offset
api_port=$(find_port $((3001 + offset * 10)))
vite_port=$(find_port $((3000 + offset * 10)))

# Generate .env.local with port overrides
cat >.env.local <<EOF
API_PORT=$api_port
VITE_PORT=$vite_port
VITE_PUBLIC_API_URL=http://localhost:$api_port
EOF

echo "Created .env.local with ports: API=$api_port, VITE=$vite_port"
```

Configure muxix to copy `.env` and generate `.env.local`:

```yaml
# .muxix.yaml
files:
  copy:
    - .env # Copy secrets (DATABASE_URL, API keys, etc.)

post_create:
  - ./scripts/worktree-env # Generate .env.local with unique ports
```

For plain Node.js (without framework support), load both files with later
overriding earlier:

```json
{
  "scripts": {
    "api": "node --env-file=.env --env-file=.env.local api/server.js",
    "web": "node --env-file=.env --env-file=.env.local web/server.js"
  }
}
```

Each worktree now gets unique ports derived from its name, allowing multiple
instances to run simultaneously without conflicts. The `.env` file stays
untouched, and `.env.local` is gitignored.

See the [Monorepos guide](https://muxix.dev/guide/monorepos) for
alternative approaches using direnv.

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

### Nerdfont icons

On first run, muxix prompts you to check if a git branch icon displays
correctly. If you have a [Nerd Font](https://www.nerdfonts.com/) installed,
answer yes to enable nerdfont icons throughout the interface, including the tmux
window prefix.

![nerdfont window prefix](https://raw.githubusercontent.com/lcensies/muxix/refs/heads/main/meta/nerdfont-prefix.webp)

To change the setting later, edit `~/.config/muxix/config.yaml`:

```yaml
nerdfont: true # or false for unicode fallbacks
```

### Using direnv

If your project uses [direnv](https://direnv.net/) for environment management,
you can configure muxix to automatically set it up in new worktrees:

```yaml
# .muxix.yaml
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

Add this to your global config (`~/.config/muxix/config.yaml`) or project's
`.muxix.yaml`. Since this file contains user-specific permissions, also add it
to `.gitignore`:

```
.claude/settings.local.json
```

**Skip permission prompts (yolo mode)**

To skip prompts entirely, define a
[named agent](https://muxix.dev/guide/agents#named-agents) that shadows
`claude`:

```yaml
# ~/.config/muxix/config.yaml
agents:
  claude: 'claude --dangerously-skip-permissions'
```

This makes all muxix-created worktrees use the flag automatically, without
affecting `claude` outside of muxix. You can also use a separate name and
reference it per-project with `agent: cc-yolo`.

### Delegating tasks with `/worktree`

The `/worktree` [skill](https://muxix.dev/guide/skills) lets you
delegate tasks to parallel worktree agents directly from your conversation. A
main agent on the main branch can act as a coordinator: planning work and
spinning up worktree agents for each task.

#### Usage

```
> /worktree Implement user authentication
> /worktree Fix the race condition in handler.go
> /worktree Add dark mode, Implement caching  # multiple tasks
```

See the [Skills guide](https://muxix.dev/guide/skills) for more skills
including `/merge`, `/rebase`, `/coordinator`, and `/open-pr`.

## Shell completions

To enable tab completions for commands and branch names, add the following to
your shell's configuration file.

For **bash**, add to your `.bashrc`:

```bash
eval "$(muxix completions bash)"
```

For **zsh**, add to your `.zshrc`:

```bash
eval "$(muxix completions zsh)"
```

For **fish**, add to your `config.fish`:

```bash
muxix completions fish | source
```

## Requirements

- Rust (for building)
- Git 2.5+ (for worktree support)
- tmux (or an alternative backend)

### Alternative backends

tmux is the primary backend. Experimental alternatives:

- **[WezTerm](https://muxix.dev/guide/wezterm)** — contributed by
  [@JeremyBYU](https://github.com/JeremyBYU).
- **[kitty](https://muxix.dev/guide/kitty)** — requires `allow_remote_control`
  and `listen_on`.
- **[Zellij](https://muxix.dev/guide/zellij)** — detected via `$ZELLIJ`.

muxix auto-detects the backend from environment variables (`$TMUX`,
`$WEZTERM_PANE`, `$KITTY_WINDOW_ID`, or `$ZELLIJ`). Session-specific variables
are checked first, so running tmux inside kitty correctly selects the tmux
backend. Set `$MUXIX_BACKEND` to override detection.

## Prior art

- [wtp](https://github.com/satococoa/wtp) — worktree creation and setup, which
  muxix extends by coupling each worktree to a tmux window.
- [claude-squad](https://github.com/smtg-ai/claude-squad),
  [vibe-kanban](https://github.com/BloopAI/vibe-kanban/) — parallel agents
  behind a dedicated TUI or kanban board, where muxix stays inside tmux.

## Contributing

Bug reports and feature suggestions are always welcome via issues or
discussions. Large and/or complex PRs, especially without prior discussion, may
not get merged. Thanks for contributing!

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup.

## Related projects

- [tmux-tools](https://github.com/raine/tmux-tools) — Collection of tmux
  utilities including file picker, smart sessions, and more
- [tmux-file-picker](https://github.com/raine/tmux-file-picker) — Pop up fzf in
  tmux to insert file paths
- [tmux-bro](https://github.com/raine/tmux-bro) — Smart tmux session manager
  that sets up project-specific sessions automatically
- [git-surgeon](https://github.com/raine/git-surgeon) — Non-interactive
  hunk-level git staging for AI agents
- [claude-history](https://github.com/raine/claude-history) — Search and view
  Claude Code conversation history with fzf
- [consult-llm](https://github.com/raine/consult-llm) — Consult other AI models
  from your agent workflow
- [tmux-agent-usage](https://github.com/raine/tmux-agent-usage) — Display AI agent
  rate limit usage in your tmux status bar
