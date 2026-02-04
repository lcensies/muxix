---
description: Manage container sandbox settings
---

# sandbox

Commands for managing container sandbox functionality.

## Subcommands

### sandbox auth

Authenticate with the agent inside the sandbox container. Run this once before using sandbox mode.

```bash
workmux sandbox auth
```

This starts an interactive session inside your configured sandbox container, allowing you to authenticate your agent. Credentials are saved to `~/.claude-sandbox.json` and `~/.claude-sandbox/`, which are separate from your host agent credentials.

## Prerequisites

Before running `sandbox auth`, you must configure sandbox in your config:

```yaml
sandbox:
  enabled: true
  image: your-sandbox-image
```

## Example

```bash
# First, configure sandbox
# In ~/.config/workmux/config.yaml or .workmux.yaml:
#   sandbox:
#     enabled: true
#     image: workmux-sandbox

# Then authenticate
workmux sandbox auth

# Output:
# Starting sandbox auth flow...
# This will open the agent in a container for authentication.
# Your credentials will be saved to ~/.claude-sandbox.json
#
# [Interactive agent session]
#
# Auth complete. Sandbox credentials saved.
```

## See also

- [Container sandbox guide](/guide/sandbox) for full setup instructions
