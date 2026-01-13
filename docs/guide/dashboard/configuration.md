---
description: Customize dashboard commands and layout
---

# Configuration

The dashboard can be customized in your `.workmux.yaml`:

```yaml
dashboard:
  commit: "Commit staged changes with a descriptive message"
  merge: "!workmux merge"
  preview_size: 60
```

The `commit` and `merge` values are text sent to the agent's pane. Use the `!` prefix to run shell commands (supported by Claude, Gemini, and other agents).

## Defaults

| Option         | Default value                                      | Description                               |
| -------------- | -------------------------------------------------- | ----------------------------------------- |
| `commit`       | `Commit staged changes with a descriptive message` | Natural language prompt                   |
| `merge`        | `!workmux merge`                                   | Shell command via agent                   |
| `preview_size` | `60`                                               | Preview pane height as percentage (10-90) |

## Preview size

The `preview_size` option controls the height of the preview pane as a percentage of the terminal height. A higher value means more space for the preview and less for the table.

You can also adjust the preview size interactively with `+`/`-` keys. These adjustments persist across dashboard sessions via tmux variables.

The CLI flag `--preview-size` (`-P`) overrides both the config and saved preference for that session.

## Examples

```yaml
# Use Claude slash commands (requires ~/.claude/commands/ setup)
dashboard:
  commit: "/commit"
  merge: "/merge"

# Custom shell commands
dashboard:
  merge: "!workmux merge --rebase --notification"

# Natural language prompts
dashboard:
  commit: "Create a commit with a conventional commit message"
  merge: "Rebase onto main and run workmux merge"
```

## Using slash commands

For complex workflows, [slash commands](/guide/slash-commands) are more powerful than simple prompts or shell commands. A slash command can encode detailed, multi-step instructions that the agent follows intelligently.

```yaml
dashboard:
  commit: "/commit"
  merge: "/merge"
```

See the [slash commands guide](/guide/slash-commands) for a complete `/merge` example you can copy.
