# AI Agents

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

## Status tracking {#status-tracking}

Workmux can display the status of the agent in your tmux window list, giving you at-a-glance visibility into what the agent in each window is doing.

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/status.webp" alt="tmux status showing agent icons" style="border-radius: 4px;">
</div>

### Status icons

- 🤖 = agent is working
- 💬 = agent is waiting for user input
- ✅ = agent finished (auto-clears on window focus)

::: info
Currently only Claude Code supports hooks that enable this functionality. Gemini's support is [on the way](https://github.com/google-gemini/gemini-cli/issues/9070). Codex support can be tracked in [this issue](https://github.com/openai/codex/issues/2109).
:::

### Setup

Install the workmux status plugin in Claude Code:

```bash
claude plugin marketplace add raine/workmux
claude plugin install workmux-status
```

Alternatively, you can manually add the hooks to `~/.claude/settings.json`. See [.claude-plugin/plugin.json](https://github.com/raine/workmux/blob/main/.claude-plugin/plugin.json) for the hook configuration.

Workmux automatically modifies your tmux `window-status-format` to display the status icons. This happens once per session and only affects the current tmux session (not your global config).

### Customization

You can customize the icons in your config:

```yaml
# ~/.config/workmux/config.yaml
status_icons:
  working: '🔄'
  waiting: '⏸️'
  done: '✔️'
```

If you prefer to manage the tmux format yourself, disable auto-modification and add the status variable to your `~/.tmux.conf`:

```yaml
# ~/.config/workmux/config.yaml
status_format: false
```

```bash
# ~/.tmux.conf
set -g window-status-format '#I:#W#{?@workmux_status, #{@workmux_status},}#{?window_flags,#{window_flags}, }'
set -g window-status-current-format '#I:#W#{?@workmux_status, #{@workmux_status},}#{?window_flags,#{window_flags}, }'
```

## Delegating tasks {#delegating-tasks}

A Claude Code [custom slash command](https://docs.anthropic.com/en/docs/claude-code/tutorials/custom-slash-commands) can streamline task delegation to worktree agents. Save this as `~/.claude/commands/worktree.md`:

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

### Usage

```bash
> /worktree Implement user authentication
> /worktree Fix the race condition in handler.go
> /worktree Add dark mode, Implement caching  # multiple tasks
```

::: tip
See the [blog post on delegating tasks](https://raine.dev/blog/git-worktrees-parallel-agents/) for a detailed walkthrough of this workflow.
:::

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
agent: 'claude --dangerously-skip-permissions'
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

See the [add command reference](/reference/commands#add) for all parallel workflow options.
