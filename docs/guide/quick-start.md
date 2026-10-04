---
description: Get started with muxix in minutes
---

# Quick start

::: info Prerequisites
muxix requires a terminal multiplexer. Make sure you have [tmux](https://github.com/tmux/tmux) (or [WezTerm](/guide/wezterm) / [Kitty](/guide/kitty) / [Zellij](/guide/zellij)) installed and running before you start. See [My tmux setup](https://raine.dev/blog/my-tmux-setup/) if you need a starting point.
:::

## 1. Install

```bash
curl -fsSL https://raw.githubusercontent.com/lcensies/muxix/main/scripts/install.sh | bash
```

See [Installation](/guide/installation) for other methods (Homebrew, Cargo, Nix).

## 2. Initialize configuration (optional)

```bash
muxix init
```

This creates a `.muxix.yaml` file to customize your workflow (pane layouts, setup commands, file operations, etc.). muxix works out of the box with sensible defaults, so this step is optional.

## 3. Create a new worktree and tmux window

```bash
muxix add new-feature
```

This will:

- Create a git worktree at `<project_root>/../<project_name>__worktrees/new-feature`
- Copy config files and symlink dependencies (if [configured](/guide/configuration#file-operations))
- Run any [`post_create`](/guide/configuration#lifecycle-hooks) setup commands
- Create a tmux window named `mx-new-feature` (the prefix is configurable)
- Set up your configured or the default tmux pane layout
- Automatically switch your tmux client to the new window

::: tip
**Highly recommended workflow**: If you're already in an agent session, use the [`/worktree` skill](/guide/skills#-worktree) instead of writing `muxix add` commands yourself:

```text
> /worktree Add pagination to the users endpoint
> /worktree Fix it <referring to some agent's question>
```

The agent writes detailed prompts based on the current context and launches each task in its own worktree.
:::

## 4. Do your thing

Work on your feature, fix a bug, or let an AI agent handle it.

## 5. Finish and clean up

**Local merge:** Run `muxix merge` to merge into the base branch and clean up in one step.

**PR workflow:** Use [`/open-pr`](/guide/skills#open-pr) to push and open a PR. After it's merged, run `muxix remove` to clean up.

See [Workflows](/guide/workflows) for more patterns including delegating tasks from agent sessions.

## Directory structure

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

Each worktree is a separate working directory for a different branch, all sharing the same git repository. This allows you to work on multiple branches simultaneously without conflicts.

You can customize the worktree directory location using the `worktree_dir` configuration option (see [Configuration](/guide/configuration)).

## Workflow example

Here's a complete workflow:

```bash
# Start a new feature
muxix add user-auth

# Work on your feature...
# (muxix automatically sets up your configured panes and environment)

# When ready, merge and clean up
muxix merge user-auth

# Start another feature
muxix add api-endpoint

# List all active worktrees
muxix list
```

## The parallel AI workflow

Run multiple AI agents simultaneously, each in its own worktree. No conflicts, no branch switching, no stashing.

```bash
# Spin up two agents working on different tasks
muxix add refactor-user-model -p "Refactor the User model to use composition"
muxix add add-search-endpoint -p "Add a /search endpoint with pagination"

# Each agent works in isolation. Check progress via tmux windows or the dashboard
muxix dashboard

# Merge completed work back to main
muxix merge refactor-user-model
muxix merge add-search-endpoint
```

::: tip
Use `-A` (`--auto-name`) to [generate branch names automatically](/reference/commands/add#automatic-branch-name-generation) from your prompt, so you don't have to think of one.
:::

See [AI Agents](/guide/agents) for details on prompts, multi-agent generation, and agent status tracking.
