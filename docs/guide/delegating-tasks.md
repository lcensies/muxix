---
description: Use a main agent to coordinate and delegate tasks to worktree agents
---

# Delegating tasks

See the [blog post on delegating tasks](https://raine.dev/blog/git-worktrees-parallel-agents/) for a detailed walkthrough.

A main agent on the main branch can act as a coordinator: planning work and delegating tasks to worktree agents.

## Slash command

A Claude Code [custom slash command](https://docs.anthropic.com/en/docs/claude-code/tutorials/custom-slash-commands) can streamline task delegation. Save this as `~/.claude/commands/worktree.md`:

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

## Usage

```bash
> /worktree Implement user authentication
> /worktree Fix the race condition in handler.go
> /worktree Add dark mode, Implement caching  # multiple tasks
```

## Customization

You can customize the slash command to add additional instructions for worktree agents. For example, to have agents review their changes with a subagent before finishing:

```markdown
The prompt file should:

- Include the full task description
- Use RELATIVE paths only (never absolute paths, since each worktree has its own
  root directory)
- Be specific about what the agent should accomplish
- Instruct the agent to review the changes with a subagent when done
```

Or if you're feeling YOLO, instruct agents to run `workmux merge` after completing their task to automatically merge changes back to the main branch.
