---
description: Manage container sandbox settings
---

# sandbox

Commands for managing container sandbox functionality.

## Subcommands

### sandbox build

Build the sandbox container image with Claude Code and workmux pre-installed.

```bash
workmux sandbox build [--force]
```

**Options:**

- `--force` - Build even on non-Linux OS (the workmux binary will not work in the image)

This builds a Docker image named `workmux-sandbox` (or your configured image name) containing Claude Code and the workmux binary. The image is built from an embedded Dockerfile template.

**Note:** This command must be run on Linux because it copies your local workmux binary into the image. On macOS/Windows, it will fail unless `--force` is used.

### sandbox auth

Authenticate with the agent inside the sandbox container. Run this once before using sandbox mode.

```bash
workmux sandbox auth
```

This starts an interactive session inside your configured sandbox container, allowing you to authenticate your agent. Credentials are saved to `~/.claude-sandbox.json` and `~/.claude-sandbox/`, which are separate from your host agent credentials.

## Quick Setup

```bash
# 1. Build the image (on Linux)
workmux sandbox build

# 2. Authenticate
workmux sandbox auth

# 3. Enable in config (~/.config/workmux/config.yaml or .workmux.yaml)
#    sandbox:
#      enabled: true
```

## Example

```bash
# Build the sandbox image
workmux sandbox build
# Output:
# Building sandbox image 'workmux-sandbox'...
# Building image 'workmux-sandbox' using docker...
# ...
# Sandbox image built successfully!

# Then authenticate
workmux sandbox auth
# Output:
# Starting sandbox auth flow...
# This will open Claude in container 'workmux-sandbox' for authentication.
# Your credentials will be saved to ~/.claude-sandbox.json
#
# [Interactive agent session]
#
# Auth complete. Sandbox credentials saved.
```

## See also

- [Container sandbox guide](/guide/sandbox) for full setup instructions
