---
description: Run agents in isolated Docker or Podman containers for enhanced security
---

# Container Sandbox

The container sandbox runs agents in isolated Docker or Podman containers, restricting their access to only the current worktree. This protects sensitive files like SSH keys, AWS credentials, and other secrets from agent access.

## Security Model

When sandbox is enabled:

- Agents can only access the current worktree directory
- The main `.git` directory is mounted read-write (for git operations)
- Sandbox uses separate authentication stored in `~/.claude-sandbox.json`
- Host credentials (SSH keys, AWS, etc.) are not accessible

## Setup

### 1. Install Docker or Podman

```bash
# macOS
brew install --cask docker

# Or for Podman
brew install podman
```

### 2. Build the sandbox image

On a **Linux machine**, run:

```bash
workmux sandbox build
```

This builds a Docker image named `workmux-sandbox` containing:

- Claude Code CLI
- The workmux binary (for status hooks)
- Git and other dependencies

**Note:** The build command must be run on Linux because it copies your local
workmux binary into the image. On macOS/Windows, the binary would be
incompatible with the Linux container.

**Alternative: Manual build**

If you need to build on a non-Linux machine or want a custom image:

```dockerfile
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    curl git ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Install Claude Code
RUN curl -fsSL https://claude.ai/install.sh | bash

# Optional: download workmux from releases
# RUN curl -fsSL https://github.com/user/workmux/releases/latest/download/workmux-linux -o /usr/local/bin/workmux && chmod +x /usr/local/bin/workmux

ENV PATH="/root/.claude/local/bin:${PATH}"
```

```bash
docker build -t workmux-sandbox .
```

### 3. Enable sandbox in config

Add to your global or project config:

```yaml
# ~/.config/workmux/config.yaml or .workmux.yaml
sandbox:
  enabled: true
  # image defaults to 'workmux-sandbox' if not specified
```

### 4. Authenticate once

The sandbox uses separate credentials from your host. Run this once to authenticate your agent inside the container:

```bash
workmux sandbox auth
```

This saves credentials to `~/.claude-sandbox.json`, which is mounted into containers.

## Configuration

| Option | Default | Description |
|--------|---------|-------------|
| `enabled` | `false` | Enable container sandboxing |
| `runtime` | `docker` | Container runtime: `docker` or `podman` |
| `target` | `agent` | Which panes to sandbox: `agent` or `all` |
| `image` | `workmux-sandbox` | Container image name |
| `env_passthrough` | `["GITHUB_TOKEN"]` | Environment variables to pass through |

### Example configurations

**Minimal:**

```yaml
sandbox:
  enabled: true
```

**With Podman and custom env:**

```yaml
sandbox:
  enabled: true
  runtime: podman
  image: my-sandbox:latest
  env_passthrough:
    - GITHUB_TOKEN
    - ANTHROPIC_API_KEY
```

**Sandbox all panes (not just agent):**

```yaml
sandbox:
  enabled: true
  target: all
```

## How It Works

When you run `workmux add feature-x`, the agent command is wrapped:

```bash
# Without sandbox:
claude -- "$(cat .workmux/PROMPT-feature-x.md)"

# With sandbox:
docker run --rm -it \
  --user 501:20 \
  --mount type=bind,source=/path/to/worktree,target=/path/to/worktree \
  --mount type=bind,source=/path/to/main/.git,target=/path/to/main/.git \
  --mount type=bind,source=~/.claude-sandbox.json,target=/root/.claude.json \
  --workdir /path/to/worktree \
  workmux-sandbox \
  sh -c 'claude -- "$(cat .workmux/PROMPT-feature-x.md)"'
```

### What's mounted

| Mount | Access | Purpose |
|-------|--------|---------|
| Worktree directory | read-write | Source code |
| Main `.git` | read-write | Git operations |
| `~/.claude-sandbox.json` | read-write | Agent config |
| `~/.claude-sandbox/` | read-write | Agent settings |

### What's NOT accessible

- `~/.ssh/` (SSH keys)
- `~/.aws/` (AWS credentials)
- `~/.config/` (other app configs)
- Other worktrees
- Main worktree source files

## Limitations

### Coordinator agents

If a coordinator agent spawns sub-agents via workmux, those sub-agents run outside the sandbox on the host. This is a fundamental limitation of the architecture. For fully sandboxed coordination, run coordinators on the host and only sandbox leaf agents.

### `workmux merge` must run on host

The `merge` command requires access to multiple worktrees, which breaks the sandbox isolation model. Always run `workmux merge` from outside the sandbox (on the host terminal).

### macOS tmux bridge

On macOS with Docker Desktop, status updates require a TCP bridge because Unix sockets don't work across the VM boundary. This is optional for basic functionality.

## Troubleshooting

### Build fails on macOS/Windows

The `workmux sandbox build` command only works on Linux because it copies your
local binary into the container. Use `--force` to build anyway (the image will
work but workmux status hooks won't function), or build manually with a
Dockerfile that downloads workmux from releases.

### Git commands fail with "not a git repository"

The main `.git` directory must be mounted. Check that your worktree has a valid `.git` file pointing to the main repository.

### Permission denied on files

The container runs as your host user (UID:GID). Ensure your image doesn't require root permissions for the agent.

### Agent can't find credentials

Run `workmux sandbox auth` to authenticate inside the container. Credentials are separate from host credentials.

## Lima VM Backend

::: info
Lima VM backend support is currently in development. This section documents the `prune` command for managing Lima VMs.
:::

workmux can use [Lima](https://lima-vm.io/) VMs for sandboxing on macOS. Over time, unused VMs can accumulate and consume significant disk space.

### Cleaning up unused VMs

Use the `prune` command to delete unused Lima VMs created by workmux:

```bash
workmux sandbox prune
```

This command:

- Lists all Lima VMs with the `wm-` prefix (workmux VMs)
- Shows details for each VM: name, status, size, age, and last accessed time
- Displays total disk space used
- Prompts for confirmation before deletion

**Force deletion without confirmation:**

```bash
workmux sandbox prune --force
```

**Example output:**

```
Found 2 workmux Lima VM(s):

1. wm-myproject-bbeb2cbf (Running)
   Size: 100.87 GB
   Age: 2 hours ago
   Last accessed: 5 minutes ago

2. wm-another-proj-d1370a2a (Stopped)
   Size: 100.87 GB
   Age: 1 day ago
   Last accessed: 1 day ago

Total disk space: 201.74 GB

Delete all these VMs? [y/N]
```

Lima VMs are stored in `~/.lima/<name>/`. Each VM typically uses 100GB of disk space by default.

### Stopping Lima VMs

When using the Lima backend, you can stop running VMs to free up system resources:

```bash
# Interactive mode - shows list of running VMs
workmux sandbox stop

# Stop a specific VM
workmux sandbox stop wm-myproject-abc12345

# Stop all workmux VMs
workmux sandbox stop --all

# Skip confirmation (useful for scripts)
workmux sandbox stop --all --yes
```

This is useful when you want to:
- Free up CPU and memory resources
- Reduce battery usage on laptops
- Clean up after finishing work

The VMs will automatically restart when needed for new worktrees.
