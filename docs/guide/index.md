# What is workmux?

workmux is a giga opinionated zero-friction workflow tool for managing [git worktrees](https://git-scm.com/docs/git-worktree) and tmux windows as isolated development environments. Perfect for running multiple AI agents in parallel without conflict.

## Philosophy

- **Native tmux integration**: Workmux creates windows in your current tmux session. Your existing shortcuts, themes, and workflow stay intact.
- **One worktree, one tmux window**: Each git worktree gets its own dedicated, pre-configured tmux window.
- **Frictionless**: Multi-step workflows are reduced to simple commands.
- **Configuration as code**: Define your tmux layout and setup steps in `.workmux.yaml`.

The core principle is that **tmux is the interface**. If you already live in tmux, you shouldn't need to learn a new TUI app or separate interface to manage your work. With workmux, managing parallel development tasks, or multiple AI agents, is as simple as managing tmux windows.

## Features

- Create git worktrees with matching tmux windows in a single command (`add`)
- Merge branches and clean up everything (worktree, tmux window, branches) in one command (`merge`)
- Automatically set up your preferred tmux pane layout (editor, shell, watchers, etc.)
- Run post-creation hooks (install dependencies, setup database, etc.)
- Copy or symlink configuration files (`.env`, `node_modules`) into new worktrees
- [Delegate tasks to worktree agents](/guide/agents#delegating-tasks) with a `/worktree` slash command
- [Automatic branch name generation](/reference/commands#automatic-branch-name-generation) from prompts using LLM
- [Display Claude agent status in tmux window names](/guide/agents#status-tracking)
- Shell completions

## Why workmux?

workmux turns a multi-step manual workflow into two simple commands, making parallel development workflows practical.

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

## Why git worktrees?

[Git worktrees](https://git-scm.com/docs/git-worktree) let you have multiple branches checked out at once in the same repository, each in a separate directory. This provides two main advantages over a standard single-directory setup:

- **Painless context switching**: Switch between tasks just by changing directories (`cd ../other-branch`). There's no need to `git stash` or make temporary commits. Your work-in-progress, editor state, and command history remain isolated and intact for each branch.

- **True parallel development**: Work on multiple branches simultaneously without interference. You can run builds, install dependencies (`npm install`), or run tests in one worktree while actively coding in another. This isolation is perfect for running multiple AI agents in parallel on different tasks.

In a standard Git setup, switching branches disrupts your flow by requiring a clean working tree. Worktrees remove this friction. `workmux` automates the entire process and pairs each worktree with a dedicated tmux window, creating fully isolated development environments.

## Requirements

- Git 2.5+ (for worktree support)
- tmux

## Related projects

- [tmux-tools](https://github.com/raine/tmux-tools) — Collection of tmux utilities including file picker, smart sessions, and more
- [tmux-file-picker](https://github.com/raine/tmux-file-picker) — Pop up fzf in tmux to quickly insert file paths, perfect for AI coding assistants
- [tmux-bro](https://github.com/raine/tmux-bro) — Smart tmux session manager that sets up project-specific sessions automatically
- [claude-history](https://github.com/raine/claude-history) — Search and view Claude Code conversation history with fzf
- [consult-llm-mcp](https://github.com/raine/consult-llm-mcp) — MCP server that lets Claude Code consult stronger AI models (o3, Gemini, GPT-5.1 Codex)
