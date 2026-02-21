---
description: Run multiple AI agents in parallel, each in their own isolated git worktree environment
---

# AI agents

workmux is designed with AI agent workflows in mind. Run multiple agents in parallel, each in their own isolated environment.

## Agent integration

When you provide a prompt via `--prompt`, `--prompt-file`, or `--prompt-editor`, workmux automatically injects the prompt into panes running the configured agent command (e.g., `claude`, `codex`, `opencode`, `gemini`, or whatever you've set via the `agent` config or `--agent` flag) without requiring any `.workmux.yaml` changes:

- Panes with a command matching the configured agent are automatically started with the given prompt.
- You can keep your `.workmux.yaml` pane configuration simple (e.g., `panes: [{ command: "<agent>" }]`) and let workmux handle prompt injection at runtime.

This means you can launch AI agents with task-specific prompts without modifying your project configuration for each task.

### Examples

```bash
# Create a worktree with an inline prompt
workmux add feature/auth -p "Implement user authentication with OAuth"

# Create a worktree with a prompt from a file
workmux add feature/refactor --prompt-file task-description.md

# Open your editor to write a prompt interactively
workmux add feature/new-api --prompt-editor

# Override the default agent for a specific worktree
workmux add feature/caching -a gemini -p "Add caching layer for API responses"

# Use -A to generate branch name from the prompt automatically
workmux add -A -p "Fix race condition in payment handler"

# Use -A alone to open editor for prompt, then generate branch name from it
workmux add -A
```

::: tip
The `-A` (`--auto-name`) flag uses an LLM to [generate a branch name](/reference/commands/add#automatic-branch-name-generation) from your prompt, so you don't have to think of one.
:::

## Per-pane agents

workmux automatically recognizes built-in agent commands (`claude`, `gemini`, `codex`, `opencode`) in pane commands. This means prompt injection works without the `<agent>` placeholder or a matching `agent` config:

```yaml
panes:
  - command: "claude --dangerously-skip-permissions"
    focus: true
  - command: "codex --yolo"
    split: vertical
```

Each agent receives the prompt using its native format (e.g., Claude uses `--`, Gemini uses `-i`). Auto-detection matches the executable name regardless of flags or path. Just provide a prompt via `-p`, `-P`, or `-e`.

See [pane configuration](/guide/configuration#agent-placeholders) for details.

## Parallel workflows

workmux can generate multiple worktrees from a single `add` command, which is ideal for running parallel experiments or delegating tasks to multiple AI agents.

### Multi-agent example

```bash
# Create one worktree for claude and one for gemini with a focused prompt
workmux add my-feature -a claude -a gemini -p "Implement the new search API integration"
# Generates worktrees: my-feature-claude, my-feature-gemini

# Create 2 instances of the default agent
workmux add my-feature -n 2 -p "Implement task #{{ num }} in TASKS.md"
# Generates worktrees: my-feature-1, my-feature-2
```

See the [add command reference](/reference/commands/add) for all parallel workflow options.
