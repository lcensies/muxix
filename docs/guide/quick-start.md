---
description: Get started with workmux in minutes
---

# Quick start

## 1. Initialize configuration (optional)

```bash
workmux init
```

This creates a `.workmux.yaml` file to customize your workflow (pane layouts, setup commands, file operations, etc.). workmux works out of the box with sensible defaults, so this step is optional.

## 2. Create a new worktree and tmux window

```bash
workmux add new-feature
```

This will:

- Create a git worktree at `<project_root>/../<project_name>__worktrees/new-feature`
- Create a tmux window named `wm-new-feature` (the prefix is configurable)
- Set up your configured or the default tmux pane layout
- Automatically switch your tmux client to the new window

## 3. Do your thing

Work on your feature, fix a bug, or let an AI agent handle it.

## 4. When done, merge and clean up

```bash
# Run in the worktree window
workmux merge
```

Merges your branch into main and cleans up everything (tmux window, worktree, and local branch).

## Directory structure

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

Each worktree is a separate working directory for a different branch, all sharing the same git repository. This allows you to work on multiple branches simultaneously without conflicts.

You can customize the worktree directory location using the `worktree_dir` configuration option (see [Configuration](/guide/configuration)).

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

## The parallel AI workflow

Run multiple AI agents simultaneously, each in its own worktree. No conflicts, no branch switching, no stashing.

```bash
# Spin up two agents working on different tasks
workmux add refactor-user-model -p "Refactor the User model to use composition"
workmux add add-search-endpoint -p "Add a /search endpoint with pagination"

# Each agent works in isolation. Check progress via tmux windows or the dashboard
workmux dashboard

# Merge completed work back to main
workmux merge refactor-user-model
workmux merge add-search-endpoint
```

::: tip
Use `-A` (`--auto-name`) to [generate branch names automatically](/reference/commands/add#automatic-branch-name-generation) from your prompt, so you don't have to think of one.
:::

See [AI Agents](/guide/agents) for details on prompts, multi-agent generation, and agent status tracking.
