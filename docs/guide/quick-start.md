# Quick Start

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

Delegate multiple complex tasks to AI agents and let them work at the same time. This workflow is cumbersome to manage manually.

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

See [AI Agents](/guide/agents) for more details on running parallel AI workflows.
