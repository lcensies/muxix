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
