---
description: Use skills to streamline workmux workflows
---

# Skills

[Claude Code skills](https://code.claude.com/docs/en/skills) extend what Claude can do. Create a `SKILL.md` file with instructions, and Claude adds it to its toolkit. Claude uses skills when relevant, or you can invoke one directly with `/skill-name`.

::: tip
This documentation uses Claude Code's skill support as example, but other agents implement similar features. For example, [OpenCode skills](https://opencode.ai/docs/skills/). Adapt to your favorite agent as needed.
:::

## Using with workmux

Skills unlock the full potential of workmux. While you can run workmux commands directly, skills let agents handle the complete workflow - committing with context-aware messages, resolving conflicts intelligently, and delegating tasks to parallel worktrees.

- [**`/merge`**](#-merge) - Commit, rebase, and merge the current branch
- [**`/rebase`**](#-rebase) - Rebase with flexible target and smart conflict resolution
- [**`/worktree`**](#-worktree) - Delegate tasks to parallel worktree agents
- [**`/open-pr`**](#-open-pr) - Write a PR description using conversation context

You can trigger `/merge` from the [dashboard](/guide/dashboard/configuration) using the `m` keybinding:

```yaml
dashboard:
  merge: "/merge"
```

## Installation

Copy the skills you want from [`skills/`](https://github.com/raine/workmux/tree/main/skills) to your skills directory:

**Claude Code**: `~/.claude/skills/` (or project `.claude/skills/`)

## `/merge`

Handles the complete merge workflow:

1. Commit staged changes using a specific commit style
2. Rebase onto the base branch with smart conflict resolution
3. Run `workmux merge` to merge, clean up, and send a notification when complete

[**View skill →**](https://github.com/raine/workmux/tree/main/skills/merge/SKILL.md)

Instead of just running `workmux merge`, this skill:

- Commits staged changes first - the agent has full context on the work done and can write a meaningful commit message
- Reviews base branch changes before resolving conflicts - the agent understands both sides and can merge intelligently
- Asks for guidance on complex conflicts

## `/rebase`

Rebases with flexible target selection and smart conflict resolution.

[**View skill →**](https://github.com/raine/workmux/tree/main/skills/rebase/SKILL.md)

Usage: `/rebase`, `/rebase origin`, `/rebase origin/develop`, `/rebase feature-branch`

See [Resolve merge conflicts with Claude Code](https://raine.dev/blog/resolve-conflicts-with-claude/) for more on this approach.

## `/worktree`

Delegates tasks to parallel worktree agents. A main agent on the main branch can act as a coordinator: planning work and delegating tasks to worktree agents.

[**View skill →**](https://github.com/raine/workmux/tree/main/skills/worktree/SKILL.md)

See the [blog post on delegating tasks](https://raine.dev/blog/git-worktrees-parallel-agents/) for a detailed walkthrough.

Usage:

```bash
> /worktree Implement user authentication
> /worktree Fix the race condition in handler.go
> /worktree Add dark mode, Implement caching  # multiple tasks
```

### Customization

You can customize the skill to add additional instructions for worktree agents. For example, to have agents review their changes with a subagent before finishing, or run `workmux merge` after completing their task.

## `/open-pr`

Writes a PR description using the conversation context and opens the PR creation page in browser. This is the recommended way to finish work in repos that use pull requests.

[**View skill →**](https://github.com/raine/workmux/tree/main/skills/open-pr/SKILL.md)

The skill is opinionated: it opens the PR creation page in your browser rather than creating the PR directly. This lets you review and edit the description before submitting.

The agent knows what it built and why, so it can write a PR description that captures that context.
