# AI agents

workmux is designed with AI agent workflows in mind. Run multiple agents in parallel, each in their own isolated environment.

## Agent integration

When you provide a prompt via `--prompt`, `--prompt-file`, or `--prompt-editor`, workmux automatically injects the prompt into panes running the configured agent command (e.g., `claude`, `codex`, `opencode`, `gemini`, or whatever you've set via the `agent` config or `--agent` flag) without requiring any `.workmux.yaml` changes:

- Panes with a command matching the configured agent are automatically started with the given prompt.
- You can keep your `.workmux.yaml` pane configuration simple (e.g., `panes: [{ command: "<agent>" }]`) and let workmux handle prompt injection at runtime.

This means you can launch AI agents with task-specific prompts without modifying your project configuration for each task.

### Examples

```bash
# Create a worktree with an inline prompt for AI agents
workmux add feature/ai --prompt "Implement user authentication with OAuth"

# Override the default agent for a specific worktree
workmux add feature/testing -a gemini

# Create a worktree with a prompt from a file
workmux add feature/refactor --prompt-file task-description.md

# Open your editor to write a prompt interactively
workmux add feature/new-api --prompt-editor
```

## Claude Code permissions

By default, Claude Code prompts for permission before running commands. There are several ways to handle this in worktrees:

### Share permissions across worktrees

To keep permission prompts but share granted permissions across worktrees:

```yaml
files:
  symlink:
    - .claude/settings.local.json
```

Add this to your global config (`~/.config/workmux/config.yaml`) or project's `.workmux.yaml`. Since this file contains user-specific permissions, also add it to `.gitignore`:

```
.claude/settings.local.json
```

### Skip permission prompts (yolo mode)

To skip prompts entirely, either configure the agent with the flag:

```yaml
agent: "claude --dangerously-skip-permissions"
```

This only affects workmux-created worktrees. Alternatively, use a global shell alias:

```bash
alias claude="claude --dangerously-skip-permissions"
```

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
