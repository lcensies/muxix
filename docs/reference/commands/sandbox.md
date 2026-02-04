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

### sandbox stop

Stop Lima VMs to free resources.

```bash
# Interactive mode - show list and select VM
workmux sandbox stop

# Stop specific VM
workmux sandbox stop <vm-name>

# Stop all workmux VMs
workmux sandbox stop --all

# Skip confirmation prompt
workmux sandbox stop --all --yes
```

**Arguments:**

- `<vm-name>` - Name of the VM to stop (optional, conflicts with `--all`)

**Options:**

- `--all` - Stop all workmux VMs (those starting with `wm-` prefix)
- `-y, --yes` - Skip confirmation prompt

This command helps you stop running Lima VMs created by workmux to free up system resources. When run without arguments, it shows an interactive list of running workmux VMs for you to choose from. The command will ask for confirmation before stopping any VMs unless `--yes` is provided.

**Notes:**

- This command only works with Lima backend and requires `limactl` to be installed
- Only running VMs are shown in interactive mode
- If a specified VM is already stopped, the command reports this and exits successfully
- Non-interactive environments (pipes, scripts) require `--all` or a specific VM name

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
