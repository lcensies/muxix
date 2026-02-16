---
description: Release notes and version history for workmux
---

<!-- skipped: v0.1.115 -->
<!-- skipped: v0.1.113 -->
<!-- skipped: v0.1.89 -->
<!-- skipped: v0.1.82 -->
<!-- skipped: v0.1.73 -->
<!-- skipped: v0.1.64 -->
<!-- skipped: v0.1.58 -->
<!-- skipped: v0.1.56 -->
<!-- skipped: v0.1.27 -->
<!-- skipped: v0.1.25 -->
<!-- skipped: v0.1.8 -->

# Changelog

## v0.1.117 (2026-02-16)

- Fixed backend detection for nested multiplexers (e.g., tmux inside kitty or
  wezterm) so workmux correctly targets the innermost multiplexer
  ([#53](https://github.com/raine/workmux/issues/53))
- Added `WORKMUX_BACKEND` environment variable to explicitly override backend
  auto-detection (accepts `tmux`, `wezterm`, or `kitty`)

## v0.1.116 (2026-02-15)

- Fixed hooks and run commands failing when they use bash-specific syntax (e.g.,
  arrays, process substitution), by using bash instead of sh for execution
  ([#52](https://github.com/raine/workmux/pull/52))

## v0.1.114 (2026-02-15)

- Sandbox: Host git identity (user.name, user.email) is now automatically
  available inside sandbox environments, so git commits from sandboxed agents
  use the correct author

## v0.1.112 (2026-02-13)

- Added sandbox support for running agents in isolated environments. Two
  backends: containers (Docker/Podman) for ephemeral sessions, and Lima VMs for
  persistent machines with built-in Nix/Devbox toolchain support. See the
  [sandbox guide](https://workmux.raine.dev/guide/sandbox/) for setup.

## v0.1.111 (2026-02-12)

- Dashboard: Added Ctrl+N/Ctrl+P as alternative keybindings for navigating
  between rows
- Fixed bash completion panic when generating completions
  ([#51](https://github.com/raine/workmux/pull/51))

## v0.1.110 (2026-02-11)

- Added kitty as an alternative terminal backend -- detected automatically when
  running inside kitty
- Improved window cleanup handling for non-tmux backends

## v0.1.109 (2026-02-09)

<!-- summary: run command, coordinator commands, light theme -->

- Added coordinator commands for scripting multi-agent workflows:
  - `send` sends text or file contents to a worktree's agent pane
  - `capture` reads the last N lines from a worktree's pane output
  - `status` shows the current state of worktree agents with elapsed time and
    git info (use `--git` for staged/unstaged indicators)
  - `wait` blocks until agents reach a target status (working, waiting, or done)
  - `run` executes a command in a worktree's pane and streams the output in real
    time.
- Dashboard now supports light theme via `theme: light` in config

## v0.1.108 (2026-02-07)

- The `list` command now shows an AGENT column displaying the status of agents
  running in each worktree (working, waiting, done icons)
- Added positional arguments to `list` for filtering by worktree handle or
  branch name
- When piping output, agent status icons are replaced with text labels for
  compatibility with scripts

## v0.1.107 (2026-02-04)

- Shell autocompletion now suggests worktree names for the `close` command
  (bash, zsh, fish) ([#47](https://github.com/raine/workmux/issues/47))

## v0.1.106 (2026-02-04)

- Fixed dashboard incorrectly showing the worktree directory name instead of the
  project name when using a custom `worktree_dir` configuration
  ([#48](https://github.com/raine/workmux/pull/48))

## v0.1.105 (2026-01-31)

- Nerdfont setup now handles read-only config files gracefully (e.g., when
  symlinked to a Nix store), showing a helpful message instead of failing

## v0.1.104 (2026-01-31)

- Added `-o`/`--open-if-exists` flag to `workmux add` for idempotent worktree
  creation: if the worktree already exists, switches to it instead of failing.

## v0.1.103 (2026-01-30)

- Dashboard now shows PR status column with number and state icon (open, merged,
  closed, draft) for each agent's worktree
- Dashboard displays CI/CD check status alongside PRs with pass/fail/pending
  icons. Enable `dashboard.show_check_counts` to show pass/total counts
- Added last-agent toggle (Tab key in dashboard, `workmux last-agent` CLI) to
  quickly switch between current and previous agent
- Added `auto_name.background` config option to always run `--auto-name` agents
  in background mode
- Improved dashboard startup performance

## v0.1.102 (2026-01-29)

- Moved internal state management from tmux-specific mechanisms to
  filesystem-based JSON storage, laying the groundwork for multi-backend support
- Added experimental WezTerm backend support. workmux auto-detects the backend
  from environment variables. See the
  [WezTerm guide](https://workmux.raine.dev/guide/wezterm) for setup
  instructions. (contributed by [@JeremyBYU](https://github.com/JeremyBYU))
- New worktrees now automatically get a symlink to a gitignored
  `CLAUDE.local.md` from your main worktree, so your local Claude Code
  instructions are available without manual setup

## v0.1.101 (2026-01-29)

- Fixed status icons breaking tmux themes that use padding spaces in window
  format strings ([#45](https://github.com/raine/workmux/pull/45))

## v0.1.100 (2026-01-26)

- Added nested config support for monorepos: place a `.workmux.yaml` in any
  subdirectory to configure that project independently. When you run workmux
  from a subdirectory, it finds the nearest config. Working directory, file
  operations, and hooks are all scoped to the config directory.
  ([#39](https://github.com/raine/workmux/issues/39))
- Added `WM_CONFIG_DIR` environment variable for hooks, pointing to the
  directory containing the `.workmux.yaml` that was used

## v0.1.99 (2026-01-24)

- Dashboard: Detect and clear working agents that have stalled, for example due
  to being interrupted
- Fixed `last-done` command intermittently failing when switching to recently
  completed agents

## v0.1.98 (2026-01-23)

- Tmux window names now use a nerdfont git branch icon as the default prefix
  when nerdfonts are available, replacing the previous "wm-" prefix. This can be
  overridden with `window_prefix`
- Fixed cleanup commands outputting noise to the terminal after merging or
  removing worktrees

## v0.1.97 (2026-01-23)

- Added bash installer script for easier installation (`curl -fsSL ... | bash`)
- Added automatic nerdfont detection with fallback icons for users without
  nerdfonts installed
- Added `last-done` command to quickly switch to recently completed agents
- Fixed race condition when running merge from inside a worktree agent

## v0.1.96 (2026-01-20)

- Reduced crate download size by excluding unnecessary files from the published
  package

## v0.1.95 (2026-01-20)

- Fixed prompts failing when branch names contain slashes
  ([#37](https://github.com/raine/workmux/issues/37))

## v0.1.94 (2026-01-17)

- Fixed dashboard commit and merge commands not working in Claude Code when
  using bash command prefix (`!`)
- Fixed dashboard commands including a literal newline that caused issues with
  OpenCode ([#35](https://github.com/raine/workmux/issues/35))
- Dashboard: Added `--diff` flag to open diff view directly for the current
  worktree, skipping the agent list

## v0.1.93 (2026-01-15)

- Added Nix flake (https://workmux.raine.dev/guide/nix)
- Fixed bash completion not passing arguments to the fallback completion
  function

## v0.1.92 (2026-01-14)

- Duplicate windows created with `open --new` are now placed immediately after
  the original window instead of at the end of the window list
- `open --new` can now be run without a name argument when inside a worktree,
  inferring the current worktree automatically

## v0.1.91 (2026-01-14)

- Fixed `merge` command failing with bare repo setups that use linked worktrees
  ([#31](https://github.com/raine/workmux/issues/31))

## v0.1.90 (2026-01-13)

- Fixed false "unmerged commits" warning when local main branch is ahead of the
  remote ([#30](https://github.com/raine/workmux/issues/30))

## v0.1.88 (2026-01-13)

- The `merge` command now works when the target branch is checked out in a
  linked worktree ([#29](https://github.com/raine/workmux/issues/29))

## v0.1.87 (2026-01-13)

- Fixed `workmux add user/feature` incorrectly treating `user` as a remote name
  instead of creating a local branch named `user/feature`
  ([#28](https://github.com/raine/workmux/issues/28))
- Fixed worktree cleanup failing to run process stop hooks by deferring
  directory deletion

## v0.1.86 (2026-01-11)

- Dashboard: Preview pane size is now configurable via config file, CLI flag
  (`--preview-size`/`-P`), or interactively with `+`/`-` keys

## v0.1.85 (2026-01-11)

- Dashboard: Selection now stays on the same agent when the list reorders due to
  status changes or sorting

## v0.1.84 (2026-01-11)

- Dashboard: Improved file list layout with full paths and right-aligned stats

## v0.1.83 (2026-01-10)

- Dashboard: Added file list sidebar to diff and patch views
- Dashboard: Added Ctrl+D/U scrolling in patch mode
- Dashboard: Improved diff coloring fallback when delta is not available

## v0.1.81 (2026-01-10)

- Dashboard: Added help screen accessible with `?` key, showing keybindings for
  each view (dashboard, diff, patch mode)
- Dashboard: Added mouse scroll support in diff views
- Dashboard: The active worktree is now highlighted with a subtle background and
  white text for easier identification
- Dashboard: Git column header shows a spinner while refreshing
- Fixed agent status not showing "working" when launching with a prompt (works
  around
  [Claude Code v2.0.77 regression](https://github.com/anthropics/claude-code/issues/17284))

## v0.1.80 (2026-01-09)

- Dashboard: Commit and merge actions are now configurable via
  `dashboard.commit` and `dashboard.merge` in your config file

## v0.1.79 (2026-01-09)

- Dashboard: Fixed patch mode showing already-staged hunks
- Dashboard: Uncommitted changes are now shown for the main worktree

## v0.1.78 (2026-01-08)

- Dashboard: Added patch mode for interactive hunk-by-hunk staging with `p` key
- Dashboard: Added hunk splitting to stage partial changes within a hunk
- Dashboard: Added ability to undo staged changes in patch mode
- Dashboard: Added hunk commenting for review workflows
- Dashboard: Added diff browsing with `d` to view uncommitted changes and `D`
  for committed changes (toggle between WIP/review views with Tab)
- Dashboard: Diffs now use delta for syntax highlighting when available, with
  fallback coloring
- Dashboard: Added filter to hide stale agents with `f` key (persists across
  sessions)
- Dashboard: Added `c` to commit and `m` to merge directly from the main view
- Dashboard: Working agents now show an animated spinner

## v0.1.77 (2026-01-07)

- Dashboard: Added git status column showing diff stats (+/- lines), conflict
  indicator, dirty state, and ahead/behind counts for each worktree
- Dashboard: Non-default base branches (not main/master) are now displayed in
  the git column
- Added `--notification` flag to `merge` command to show a system notification
  on successful merge

## v0.1.76 (2026-01-07)

- Dashboard: Renamed "Agent" column to "Worktree" for clarity; non-workmux
  agents now display "main" instead of their window name
- The `merge` command now auto-detects the base branch from when the worktree
  was created, instead of always defaulting to main
- Window status icons for "waiting" and "done" again auto-clear when returning
  to the pane

## v0.1.75 (2026-01-06)

- Added OpenCode support for agent status tracking in tmux window names
- Fixed passing prompt to OpenCode

## v0.1.74 (2026-01-06)

- Dashboard: Stale agents (inactive for over an hour) now show a timer icon
  instead of "stale"
- Dashboard: Preview updates are now faster in input mode

## v0.1.72 (2026-01-06)

- Renamed "status popup" to "dashboard"

## v0.1.71 (2026-01-06)

- Added pane preview to the status dashboard, showing live terminal output from
  the selected agent
- Added input mode: press `i` to send keystrokes directly to the selected
  agent's pane without switching windows, press Escape to exit
- Added preview scrolling with Ctrl+U/D
- Agents are now automatically removed from the status list when they exit
- Priority sorting now uses elapsed time as a tiebreaker

## v0.1.70 (2026-01-06)

- Added smart sorting to the status dashboard with four modes: Priority (by
  status importance), Project (grouped by project), Recency (newest first), and
  Natural (tmux order). Press `s` to cycle through modes; it is saved across
  sessions.

## v0.1.69 (2026-01-05)

- Added `status` command: a TUI dashboard for monitoring all active agents
  across tmux sessions, with quick-jump keys (1-9), peek mode, and keyboard
  navigation
- The "done" (✅) status no longer gets replaced by "waiting" (💬) when Claude
  sends idle prompts, so completed sessions stay marked as done

## v0.1.68 (2026-01-05)

- Added `docs` command to view the README

## v0.1.67 (2026-01-04)

- Improved compatibility with non-POSIX shells like nushell
- Commands for starting agent with a prompt no longer pollute shell history

## v0.1.66 (2026-01-03)

- Added `--no-verify` (`-n`) flag to `merge` command to skip pre-merge hooks
- The `merge` command now works when run from subdirectories within a worktree

## v0.1.65 (2026-01-02)

- The `open` command now switches to an existing window by default instead of
  erroring when a window already exists
- Added `--new` (`-n`) flag to `open` command to force opening a duplicate
  window (creates suffix like `-2`, `-3`)
- The `open` command now supports prompts via `-p`, `-P`, and `-e` flags,
  matching the `add` command

## v0.1.63 (2026-01-02)

- Linux binaries now use musl for better compatibility across different Linux
  distributions

## v0.1.62 (2025-12-29)

- The `merge` command with `--keep` no longer requires a clean worktree, since
  the worktree won't be deleted anyway

## v0.1.61 (2025-12-27)

- Log files are now stored in the XDG state directory
  (`~/.local/state/workmux/`)

## v0.1.60 (2025-12-26)

- Added `close` command to close a worktree's tmux window while keeping the
  worktree on disk. It's basically an alias for tmux's `kill-window`

## v0.1.59 (2025-12-26)

- Added `pre_merge` hook to run commands (like tests or linters) before merging,
  allowing you to catch issues before they land in your main branch
- Added `pre_remove` hook that runs before worktree removal, with environment
  variables (`WM_HANDLE`, `WM_WORKTREE_PATH`, `WM_PROJECT_ROOT`) for backup or
  cleanup workflows
- The `post_create` hook now receives `WM_WORKTREE_PATH` and `WM_PROJECT_ROOT`
  environment variables, matching the other hooks

## v0.1.57 (2025-12-23)

- Fixed terminal input not being displayed after creating a worktree with
  `workmux add` on bash ([#17](https://github.com/raine/workmux/pull/17))

## v0.1.55 (2025-12-21)

- The `merge` command now allows untracked files in the target worktree, only
  blocking when there are uncommitted changes to tracked files

## v0.1.54 (2025-12-17)

- The `remove` command now accepts multiple worktree names, allowing you to
  clean up several worktrees in a single command (e.g.,
  `workmux rm feature-a feature-b`)

## v0.1.53 (2025-12-17)

- Added JSON lines support for stdin input: pipe JSON objects to `workmux add`
  and each key automatically becomes a template variable, making it easy to use
  structured data from tools like `jq` in prompts and branch names
- Template errors now show which variables are missing and list available ones,
  helping catch typos in branch name templates or prompts before worktrees are
  created
- Fixed "directory already exists" errors when creating worktrees after a
  previous cleanup was interrupted by background processes recreating files

## v0.1.52 (2025-12-17)

- Added `--max-concurrent` flag to limit how many worktrees run simultaneously,
  useful for creating worker pools that process items without overwhelming
  system resources or hitting API rate limits
- Added `{{ index }}` template variable for branch names and prompts in
  multi-worktree modes, providing a 1-indexed counter across all generated
  worktrees

## v0.1.51 (2025-12-16)

- Added `--wait` (`-W`) flag to `add` command to block until the created tmux
  window is closed, useful for scripting workflows
- Added stdin input support for multi-worktree generation: pipe lines to
  `workmux add` to create multiple worktrees, with each line available as
  `{{ input }}` in prompts
- Fixed duplicate remote fetch when using `--pr` or fork branch syntax
  (`user:branch`)

## v0.1.50 (2025-12-15)

- Fixed a crash in `workmux completions bash`
  ([#14](https://github.com/raine/workmux/issues/14))

## v0.1.49 (2025-12-15)

- Added `--all` flag to `remove` command to remove all worktrees at once (except
  the main worktree), with safety checks for uncommitted changes and unmerged
  commits
- Now shows an error when using `-p`/`--prompt` without an agent pane
  configured, instead of silently ignoring the prompt

## v0.1.48 (2025-12-10)

- Removed automatic `node_modules` symlink default for Node.js projects

## v0.1.47 (2025-12-09)

- Added `--gone` flag to `rm` command to clean up worktrees whose remote
  branches have been deleted (e.g., after PRs are merged)

## v0.1.46 (2025-12-09)

- Added `--pr` flag to `list` command to show PR status alongside worktrees,
  displaying PR numbers and state icons (open, draft, merged, closed)
- Added spinner feedback for slow operations like GitHub API calls

## v0.1.45 (2025-12-08)

- Shell completions now suggest proper values for `--base`, `--into`, and
  `--prompt-file` flags (bash, zsh)
- Fixed an error with the `pre_delete` hook when removing worktrees that were
  manually deleted from the filesystem

## v0.1.44 (2025-12-06)

- In agent status tracking, the "waiting" (💬) status icon now auto-clears
  window is focused, matching the behavior of the "done" (✅️) status.

## v0.1.43 (2025-12-05)

- Improved the default config template generated by `workmux init`

## v0.1.42 (2025-12-05)

- Added pre-built binaries for Linux ARM64 (aarch64) architecture

## v0.1.41 (2025-12-04)

- Commands `open`, `path`, `remove`, and `merge` now accept worktree names (the
  directory name shown in tmux) in addition to branch names, making it easier to
  work with worktrees when the directory name differs from the branch

## v0.1.40 (2025-12-03)

- Added `--auto-name` (`-A`) flag to automatically generate branch names from
  your prompt using an LLM (uses the `llm` tool), so you can skip naming
  branches yourself
- Added `auto_name.model` and `auto_name.system_prompt` config options to
  customize the LLM model and prompt used for branch name generation

## v0.1.39 (2025-12-03)

- New worktree windows are now inserted after the last workmux window instead of
  at the end of the window list, keeping your worktree windows grouped together

## v0.1.38 (2025-12-03)

- Fixed branches created with `--base` not having upstream tracking
  configuration properly unset from the base branch

## v0.1.37 (2025-12-03)

- Fixed panes not loading shell profiles, which broke tools like nvm etc. that
  depend on login shell initialization

## v0.1.36 (2025-12-01)

- Added `--into` flag to `merge` command for merging into branches other than
  main (e.g., `workmux merge feature --into develop`)
- Fixed config loading and file operations when running commands from inside a
  worktree
- Removed `--delete-remote` flag from `merge` and `remove` commands

## v0.1.35 (2025-12-01)

- Added agent status tracking in tmux window names, showing icons for different
  Claude Code states (🤖 working, 💬 waiting, ✅ done). The "done" status
  auto-clears when you focus the window.

## v0.1.34 (2025-11-30)

- Fixed worktree path calculation when running `add` from inside an existing
  worktree, which previously created nested paths instead of sibling worktrees

## v0.1.33 (2025-11-30)

- Added support for GitHub fork branch format (`user:branch`) in `add` command,
  allowing direct checkout of fork branches copied from GitHub's UI

## v0.1.32 (2025-11-30)

- Added OpenCode agent support: prompts are now automatically passed using the
  `-p` flag when using `--prompt-file` or `--prompt-editor` with
  `--agent opencode`

## v0.1.31 (2025-11-29)

- Added `path` command to get the filesystem path of a worktree by branch name
- Added `--name` flag to `add` command for explicit worktree directory and tmux
  window naming
- Added `worktree_naming` config option to control how worktree names are
  derived from branches (`full` or `basename`)
- Added `worktree_prefix` config option to add a prefix to all worktree
  directory names
- Added `merge_strategy` config option to set default merge behavior (merge,
  rebase, or squash)

## v0.1.30 (2025-11-27)

- Added nushell support for pane startup commands
- Improved reliability of pane command execution across different shells

## v0.1.29 (2025-11-26)

- Shell completions now suggest git branch names when using the `add` command

## v0.1.28 (2025-11-26)

- Shell completions now dynamically suggest branch names when pressing TAB for
  `open`, `merge`, and `remove` commands (bash, zsh, fish)

## v0.1.26 (2025-11-25)

- Added `--pr` flag to checkout a GitHub pull request directly into a new
  worktree
- Fixed version managers (nvm, pnpm, mise, etc.) being shadowed by stale PATH
  entries when running pane commands
- Improved list output with cleaner table formatting and relative paths
- Fixed duplicate command announcement when running merge workflow

## v0.1.24 (2025-11-22)

- Fixed "can't find pane: 0" errors when using `pane-base-index 1` in tmux
  configuration
- Merge conflicts now abort cleanly, keeping your main worktree in a usable
  state with guidance on how to resolve

## v0.1.23 (2025-11-22)

- Added `--keep` flag to merge command to merge without cleaning up the
  worktree, useful for verifying the merge before removing the branch
- Fixed a bug where multi-agent worktrees had incorrect agent configuration for
  worktrees after the first one
- After closing a worktree (merge or remove), the terminal now navigates back to
  the main worktree instead of staying in the deleted directory

## v0.1.22 (2025-11-21)

- Added YAML frontmatter support in prompt files for defining variable matrices
  (`foreach`), making it easier to specify multi-worktree generation without CLI
  flags
- Added `size` and `percentage` options for pane configuration to control pane
  dimensions when splitting
- Fixed prompt editor temporary file now using `.md` extension for better editor
  syntax highlighting
- Fixed Gemini agent startup issues

## v0.1.21 (2025-11-18)

- Switched templating engine from Tera to MiniJinja (Jinja2-compatible) for
  branch names and prompts. Existing templates should work unchanged.

## v0.1.20 (2025-11-18)

- Fixed prompts starting with a dash (e.g. "- foo") being incorrectly
  interpreted as CLI flags
- The `rm` command now automatically uses the correct base branch that was used
  when the worktree was created, instead of defaulting to the main branch

## v0.1.19 (2025-11-17)

- Added `--with-changes` flag to `add` command: move uncommitted changes from
  your current worktree to a new one, useful when you've started working on the
  wrong branch
- Added `--patch` flag: interactively select which changes to move when using
  `--with-changes`
- Added `--include-untracked` (`-u`) flag: include untracked files when moving
  changes

## v0.1.18 (2025-11-17)

- New branches now default to branching from your currently checked out branch
  instead of the main branch's remote tracking branch
- Removed the `--from-current` flag (no longer needed since this is now the
  default behavior)

## v0.1.17 (2025-11-17)

- Added multi-agent workflows: create multiple worktrees from a single command
  using `-a agent1 -a agent2`, `-n count`, or `--foreach` matrix options
- Added background mode (`-b`, `--background`) to create worktrees without
  switching to them
- Added support for prompt templating with variables like `{{ agent }}`,
  `{{ num }}`, and custom `--foreach` variables
- Added `--branch-template` option to customize generated branch names

## v0.1.16 (2025-11-16)

- Added `--prompt-editor` (`-e`) flag to write prompts using your `$EDITOR`
- Added configurable agent support with `--agent` (`-a`) flag and config option
- Added flags to skip setup steps: `--no-hooks`, `--no-file-ops`,
  `--no-pane-cmds`
- Defaulted to current branch as base for `workmux add` (errors on detached HEAD
  without explicit `--base`)
- Fixed aliases containing `<agent>` placeholder not resolving correctly

## v0.1.15 (2025-11-15)

- Added `--prompt` (`-p`) and `--prompt-file` (`-P`) options to `workmux add`
  for attaching a prompt to new worktrees
- Added `--keep-branch` (`-k`) option to `workmux remove` to preserve the local
  branch while removing the worktree and tmux window

## v0.1.14 (2025-11-14)

- Added `--base` option to specify a base branch, commit, or tag when creating a
  new worktree
- Added `--from-current` (`-c`) flag to use the current branch as the base,
  useful for stacking feature branches
- Added support for creating worktrees from remote branches (e.g.,
  `workmux add origin/feature-branch`)
- Added support for copying directories (not just files) in file operations

## v0.1.13 (2025-11-13)

- Fixed `merge` and `remove` commands failing when run from within the worktree
  being deleted
- Added safety check to prevent accidentally deleting a branch that's checked
  out in the main worktree
- Fixed pane startup commands not loading shell environment tools (like direnv,
  nvm, rbenv) before running

## v0.1.11 (2025-11-11)

- Added `pre_delete` hooks that run before worktree deletion, with automatic
  detection of Node.js projects to fast-delete `node_modules` directories in the
  background
- Pane commands now keep an interactive shell open after completion, and panes
  can be created without a command (just a shell)
- Added `target` option for panes to split from any existing pane, not just the
  most recent one
- Tmux panes now use login shells for consistent environment across all panes
- The `create` command now displays which base branch was used
- Improved validation for pane configurations with helpful error messages

## v0.1.10 (2025-11-09)

- Post-create hooks now run before the tmux window opens, so the new window
  appears ready to use instead of showing setup commands running

## v0.1.9 (2025-11-09)

- Fixed cleanup when removing a worktree from within its own tmux window

## v0.1.7 (2025-11-09)

- Fixed a race condition where cleaning up a worktree could fail if the tmux
  window hadn't fully closed yet

## v0.1.6 (2025-11-09)

- Automatically run `pnpm install` when creating worktrees in pnpm projects

## v0.1.5 (2025-11-08)

- Fixed global config to always load from `~/.config/workmux/` instead of
  platform-specific locations (e.g., `~/Library/Application Support/` on macOS)

## v0.1.4 (2025-11-07)

- Added `--from` flag to `add` command to specify which branch, commit, or tag
  to branch from
- Fixed `rm` command failing when run from within the worktree being removed
- New worktree branches no longer track a remote upstream by default

## v0.1.3 (2025-11-06)

- Added global configuration support with XDG compliance—you can now set shared
  defaults in `~/.config/workmux/config.yaml` that apply across all projects
- Project configs can inherit from global settings using `<global>` placeholder
  in lists
- After merging or removing a worktree, automatically switches to the main
  branch tmux window if it exists
- Fixed an issue where removing a worktree could fail if the current directory
  was inside that worktree

## v0.1.2 (2025-11-05)

- Fixed `prune` command to correctly parse Claude Code's config file structure

## v0.1.1 (2025-11-05)

Initial release.

- Added `open` command to switch to an existing worktree's tmux window
- Added `--rebase` and `--squash` merge strategies to the `merge` command
- Added `claude prune` command to clean up stale worktree entries from Claude's
  config
- Added configurable window name prefix via `window_prefix` setting
- Allowed `remove` command to work without arguments to remove the current
  branch
- Shell completion now works with command aliases
- Fixed merge command not cleaning up worktrees after merging
- Fixed worktree deletion issues when running from within the worktree
- Fixed new branches being incorrectly flagged as unmerged
