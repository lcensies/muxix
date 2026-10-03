---
description: Recommended patterns for starting worktrees and delegating tasks to agents
---

# Workflows

Common patterns for working with muxix and AI agents.

## Starting work

### From the terminal

When starting a new task from scratch, use `muxix add -A` (`--auto-name`):

```bash
muxix add -A
```

This opens your `$EDITOR` where you describe the task. After saving, muxix generates a branch name from your prompt and creates the worktree with the prompt passed to the agent.

It's essentially a streamlined version of `muxix add <branch-name>`, then waiting for the agent to start, then typing the prompt. But you write the prompt first and skip thinking of a branch name.

::: tip
The `-A` flag requires the [`llm`](https://llm.datasette.io/) CLI tool to be installed and configured. See [Automatic branch name generation](/reference/commands/add#automatic-branch-name-generation) for setup.

Combine with `-b` (`--background`) to launch the worktree without switching to it.
:::

You can also pass the prompt inline or from a file:

```bash
# Inline prompt
muxix add -A -p "Add pagination to the /users endpoint"

# From a file
muxix add -A -P task-spec.md
```

### From an ongoing agent session

When you're already working with an agent and want to spin off a task into a separate worktree, use the [`/worktree` skill](/guide/skills#-worktree). The agent has context on what you've discussed, so it can write a detailed prompt for the new worktree agent.

```
> /worktree Implement the caching layer we discussed
```

The main agent writes a prompt file with all the relevant context and runs `muxix add` to create the worktree. This is useful when:

- The agent already understands the task from your conversation
- You want to parallelize work while continuing in the main window
- You're delegating multiple related tasks from a plan

Add `--fork` to pass the current conversation to the new worktree agent, so it can resume with full context instead of starting from a written prompt alone:

```
> /worktree --fork Implement the caching layer we discussed
```

This pattern naturally leads to a **coordinator agent** workflow: an agent on the main branch that plans work and delegates tasks to worktree agents via `/worktree`. The coordinator stays on main and doesn't write code itself; it breaks down a larger goal into parallel tasks and spins up worktree agents to handle each one.

See [Skills](/guide/skills#-worktree) for the skill setup.

### Forking conversations

When you want a new worktree agent to pick up where the current conversation left off, use `--fork` (currently Claude Code only):

```bash
muxix add -A --fork
```

This copies the most recent conversation from the current worktree into the new one and launches the agent with `--resume`, so it has full context of what was discussed. Useful when:

- You want to branch off a conversation to explore an alternative approach
- The current agent has built up context you want to preserve in a new worktree
- You're splitting a large task and want each worktree to start with shared context

To fork a specific session (not the most recent), use `--fork=<session-id>` with the session UUID or a prefix:

```bash
muxix add my-branch --fork=abc123
```

Currently supports Claude Code conversations. The forked conversation files are copied (not moved), so the original remains unchanged.

### Driving many agents from a harness

muxix performs single actions and reports state; deciding what runs next is your
harness's job. The pieces it gives a harness are:

```bash
muxix task list --ready --json    # what is runnable
muxix add -b -P prompt.md         # start one agent in the background
muxix status --json               # who is working, waiting, or done
muxix wait --status done          # block until agents settle
muxix capture / muxix send        # read output, send follow-ups
muxix merge                       # land a finished branch
```

A harness loop is then: read the frontier, `add` a worktree per task, poll
`status`, and `merge` what passes. See [`muxix task`](/reference/commands/task)
for the graph itself.

## Finishing work

How you finish depends on whether you merge locally or use pull requests.

### Direct merge

When you want to merge directly without a pull request, use `/merge` to commit, rebase, and merge in one step:

```
> /merge
```

This slash command handles the full workflow: committing staged changes, rebasing onto main, resolving conflicts if needed, and running `muxix merge` to clean up.

If you need to sync with main before you're ready to merge (e.g., to pick up changes from other merged branches), use `/rebase`:

```
> /rebase
```

See [Skills](/guide/skills) for the skill setup.

### PR-based

If your team uses pull requests for code review, the merge happens on the remote after review. Push your branch and clean up after the PR is merged.

After committing your changes, push and create a PR. If you're working with an agent, consider using a slash command like `/open-pr` that can write the PR description using the conversation context:

```
> /open-pr
```

See [`skills/open-pr`](https://github.com/lcensies/muxix/tree/main/skills/open-pr/SKILL.md) for an example skill you can adapt.

Or manually:

```bash
git push -u origin feature-123
gh pr create
```

Once your PR is merged on GitHub, use `muxix remove` to clean up:

```bash
# Remove a specific worktree
muxix remove feature-123

# Or clean up all worktrees whose remote branches were deleted
muxix rm --gone
```

The `--gone` flag is particularly useful - it automatically finds worktrees whose upstream branches no longer exist (because the PR was merged and the branch was deleted on GitHub) and removes them.
