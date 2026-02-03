---
description: Use Claude slash commands to streamline workmux workflows
---

# Slash commands

[Claude slash commands](https://docs.anthropic.com/en/docs/claude-code/slash-commands) are markdown files in `~/.claude/commands/` that define reusable workflows. When you type `/command-name` in Claude, it expands to the full prompt.

::: tip
This documentation uses Claude Code's command support as example, but some other agents implement commands as well. For example, [opencode](https://opencode.ai/docs/commands/). Adapt to your favorite agent as needed.
:::

## Using with workmux

Slash commands pair well with workmux workflows:

- [**`/merge`**](#example-merge-command) - Commit, rebase, and merge the current branch
- [**`/rebase`**](#example-rebase-command) - Rebase with flexible target and smart conflict resolution
- [**`/commit`**](#example-commit-command) - Commit staged changes with your preferred style
- [**`/worktree`**](#example-worktree-command) - Delegate tasks to parallel worktree agents

You can trigger these from the [dashboard](/guide/dashboard/configuration) using the `c` and `m` keybindings:

```yaml
dashboard:
  commit: "/commit"
  merge: "/merge"
```

## Example: /merge command

This is the `/merge` command the author uses. It handles the complete merge workflow:

1. Commit staged changes using a specific commit style
2. Rebase onto the base branch with smart conflict resolution
3. Run `workmux merge` to merge, clean up, and send a notification when complete

Save this as `~/.claude/commands/merge.md`:

````markdown
Commit, rebase, and merge the current branch.

This command finishes work on the current branch by:

1. Committing any staged changes
2. Rebasing onto the base branch
3. Running `workmux merge` to merge and clean up

## Step 1: Commit

If there are staged changes, commit them. Use lowercase, imperative mood, no conventional commit prefixes. Skip if nothing is staged.

## Step 2: Rebase

Get the base branch from git config:

```
git config --local --get "branch.$(git branch --show-current).workmux-base"
```

If no base branch is configured, default to "main".

Rebase onto the local base branch (do NOT fetch from origin first):

```
git rebase <base-branch>
```

IMPORTANT: Do NOT run `git fetch`. Do NOT rebase onto `origin/<branch>`. Only rebase onto the local branch name (e.g., `git rebase main`, not `git rebase origin/main`).

If conflicts occur:

- BEFORE resolving any conflict, understand what changes were made to each
  conflicting file in the base branch
- For each conflicting file, run `git log -p -n 3 <base-branch> -- <file>` to
  see recent changes to that file in the base branch
- The goal is to preserve BOTH the changes from the base branch AND our branch's
  changes
- After resolving each conflict, stage the file and continue with
  `git rebase --continue`
- If a conflict is too complex or unclear, ask for guidance before proceeding

## Step 3: Merge

Run: `workmux merge --rebase --notification`

This will merge the branch into the base branch and clean up the worktree and
tmux window.
````

### Why this works well

Instead of just running `workmux merge`, this command:

- Commits staged changes first
- Reviews base branch changes before resolving conflicts
- Follows your commit style
- Asks for guidance on complex conflicts

## Example: /rebase command

A rebase command that resolves conflicts by first understanding changes in the target branch. Save as `~/.claude/commands/rebase.md`:

```markdown
Rebase the current branch.

Arguments: $ARGUMENTS

Behavior:

- No arguments: rebase on local main
- "origin": fetch origin, rebase on origin/main
- "origin/branch": fetch origin, rebase on origin/branch
- "branch": rebase on local branch

Steps:

1. Parse arguments:
   - No args → target is "main", no fetch
   - Contains "/" (e.g., "origin/develop") → split into remote and branch, fetch
     remote, target is remote/branch
   - Just "origin" → fetch origin, target is "origin/main"
   - Anything else → target is that branch name, no fetch
2. If fetching, run: `git fetch <remote>`
3. Run: `git rebase <target>`
4. If conflicts occur, handle them carefully (see below)
5. Continue until rebase is complete

Handling conflicts:

- BEFORE resolving any conflict, understand what changes were made to each
  conflicting file in the target branch
- For each conflicting file, run `git log -p -n 3 <target> -- <file>` to see
  recent changes to that file in the target branch
- The goal is to preserve BOTH the changes from the target branch AND our
  branch's changes
- After resolving each conflict, stage the file and continue with
  `git rebase --continue`
- If a conflict is too complex or unclear, ask for guidance before proceeding
```

Usage: `/rebase`, `/rebase origin`, `/rebase origin/develop`, `/rebase feature-branch`

See [Resolve merge conflicts with Claude Code](https://raine.dev/blog/resolve-conflicts-with-claude/) for more on this approach.

## Example: /commit command

A commit command that follows a consistent style. Save as `~/.claude/commands/commit.md`:

```markdown
Commit the changes using this style:

- lowercase
- imperative mood
- concise, no conventional commit prefixes
- optionally use a context prefix when it adds clarity (e.g., "docs:", "cli:")

If nothing is staged, stage all changes first.
```

## Example: /worktree command

See the [blog post on delegating tasks](https://raine.dev/blog/git-worktrees-parallel-agents/) for a detailed walkthrough.

A main agent on the main branch can act as a coordinator: planning work and delegating tasks to worktree agents. Save this as `~/.claude/commands/worktree.md`:

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

Usage:

```bash
> /worktree Implement user authentication
> /worktree Fix the race condition in handler.go
> /worktree Add dark mode, Implement caching  # multiple tasks
```

### Customization

You can customize the slash command to add additional instructions for worktree agents. For example, to have agents review their changes with a subagent before finishing:

```markdown
The prompt file should:

(...)

- Instruct the agent to review the changes with a subagent when done
```

Or if you're feeling YOLO, instruct agents to run `workmux merge` after completing their task to automatically merge changes back to the main branch.
