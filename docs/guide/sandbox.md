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

Create a Dockerfile with your agent installed:

```dockerfile
FROM ubuntu:24.04

# Install dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    curl git tmux socat ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Install your agent (e.g., Claude Code)
RUN curl -fsSL https://install.example.com | bash

# Optional: Install workmux for status hooks
# COPY workmux /usr/local/bin/workmux
```

Build it:

```bash
docker build -t workmux-sandbox .
```

### 3. Enable sandbox in config

Add to your global or project config:

```yaml
# ~/.config/workmux/config.yaml or .workmux.yaml
sandbox:
  enabled: true
  image: workmux-sandbox
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
| `image` | (required) | Container image name |
| `env_passthrough` | `["GITHUB_TOKEN"]` | Environment variables to pass through |

### Example configurations

**Minimal:**

```yaml
sandbox:
  enabled: true
  image: workmux-sandbox
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
  image: workmux-sandbox
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

### "Sandbox enabled but no image configured"

Add `image:` to your sandbox config:

```yaml
sandbox:
  enabled: true
  image: your-image-name
```

### Git commands fail with "not a git repository"

The main `.git` directory must be mounted. Check that your worktree has a valid `.git` file pointing to the main repository.

### Permission denied on files

The container runs as your host user (UID:GID). Ensure your image doesn't require root permissions for the agent.

### Agent can't find credentials

Run `workmux sandbox auth` to authenticate inside the container. Credentials are separate from host credentials.
