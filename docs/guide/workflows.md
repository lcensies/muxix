---
description: Recommended patterns for starting worktrees and delegating tasks to agents
---

# Workflows

Common patterns for working with workmux and AI agents.

## Starting work

### From the terminal

When starting a new task from scratch, use `workmux add -A` (`--auto-name`):

```bash
workmux add -A
```

This opens your `$EDITOR` where you describe the task. After saving, workmux generates a branch name from your prompt and creates the worktree with the prompt passed to the agent.

It's essentially a streamlined version of `workmux add <branch-name>`, then waiting for the agent to start, then typing the prompt. But you write the prompt first and skip thinking of a branch name.

You can also pass the prompt inline or from a file:

```bash
# Inline prompt
workmux add -A -p "Add pagination to the /users endpoint"

# From a file
workmux add -A -P task-spec.md
```

### From an ongoing agent session

When you're already working with an agent and want to spin off a task into a separate worktree, use the [`/worktree` slash command](/guide/delegating-tasks). The agent has context on what you've discussed, so it can write a detailed prompt for the new worktree agent.

```
> /worktree Implement the caching layer we discussed
```

The main agent writes a prompt file with all the relevant context and runs `workmux add` to create the worktree. This is useful when:

- The agent already understands the task from your conversation
- You want to parallelize work while continuing in the main window
- You're delegating multiple related tasks from a plan

See [Delegating tasks](/guide/delegating-tasks) for the slash command setup.

## Finishing work

When an agent completes its task, use `/merge` to commit, rebase, and merge in one step:

```
> /merge
```

This slash command handles the full workflow: committing staged changes, rebasing onto main, resolving conflicts if needed, and running `workmux merge` to clean up.

If you need to sync with main before you're ready to merge (e.g., to pick up changes from other merged branches), use `/rebase`:

```
> /rebase
```

See [Slash commands](/guide/slash-commands) for the command setup.
