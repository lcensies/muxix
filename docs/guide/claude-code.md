---
description: Configure Claude Code permissions and settings for use with workmux worktrees
---

# Claude Code

## Permissions

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

## Continuing a conversation in a worktree

Sometimes you want to continue a Claude conversation inside a worktree. Since worktrees are technically separate project directories, Claude Code treats them as different projects, so you cannot directly resume a conversation that started in another worktree (or the main tree).

[claude-history](https://github.com/raine/claude-history) solves this with its cross-project fork feature. Use `--global` mode to search conversations across all projects, then fork the one you want:

```sh
claude-history --global
```

Select a conversation and press `Ctrl+F` to fork it. When the conversation belongs to a different project than your current directory, claude-history automatically copies the session files into the current project and resumes there. The forked conversation then lives in the worktree as if it started there.
