# Configuration

The commit (`c`) and merge (`m`) actions can be customized in your `.workmux.yaml`:

```yaml
dashboard:
  commit: "Commit staged changes with a descriptive message"
  merge: "!workmux merge"
```

Both values are text sent to the agent's pane. Use the `!` prefix to run shell commands (supported by Claude, Gemini, and other agents).

## Defaults

| Action   | Default value                                      | Description             |
| -------- | -------------------------------------------------- | ----------------------- |
| `commit` | `Commit staged changes with a descriptive message` | Natural language prompt |
| `merge`  | `!workmux merge`                                   | Shell command via agent |

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
