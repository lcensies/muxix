---
description: Run agents in isolated Docker or Podman containers
---

# Container backend

The container sandbox runs agents in isolated Docker or Podman containers, providing lightweight, ephemeral environments that reset after every session.

## Setup

### 1. Install Docker or Podman

```bash
# macOS
brew install --cask docker

# Or for Podman
brew install podman
```

### 2. Enable sandbox in config

Add to your global or project config:

```yaml
# ~/.config/workmux/config.yaml or .workmux.yaml
sandbox:
  enabled: true
```

The pre-built image (`ghcr.io/raine/workmux-sandbox:{agent}`) is pulled automatically on first run based on your configured agent. No manual build step is needed.

To pull the latest image explicitly:

```bash
workmux sandbox pull
```

To build locally instead of pulling:

```bash
workmux sandbox build
```

## Configuration

| Option | Default | Description |
| --- | --- | --- |
| `enabled` | `false` | Enable container sandboxing |
| `container.runtime` | auto-detect | Container runtime: `docker` or `podman`. Auto-detected from PATH when not set (prefers docker). |
| `target` | `agent` | Which panes to sandbox: `agent` or `all` |
| `image` | `ghcr.io/raine/workmux-sandbox:{agent}` | Container image name (auto-resolved from configured agent). **Global config only.** |
| `rpc_host` | auto | Override hostname for guest-to-host RPC. Defaults to `host.docker.internal` (Docker) or `host.containers.internal` (Podman). Useful for non-standard networking setups. **Global config only.** |
| `env_passthrough` | `[]` | Environment variables to pass through. **Global config only.** |
| `extra_mounts` | `[]` | Additional host paths to mount (see [shared features](./features#extra-mounts)). **Global config only.** |
| `network.policy` | `allow` | Network restriction policy: `allow` (no restrictions) or `deny` (block all except allowed domains). See [network restrictions](./features#network-restrictions). **Global config only.** |
| `network.allowed_domains` | `[]` | Allowed outbound HTTPS domains when policy is `deny`. Supports exact matches and `*.` wildcard prefixes. **Global config only.** |

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
  image: my-sandbox:latest
  env_passthrough:
    - GITHUB_TOKEN
    - ANTHROPIC_API_KEY
  container:
    runtime: podman
```

**Sandbox all panes (not just agent):**

```yaml
sandbox:
  enabled: true
  target: all
```

## How it works

When you run `workmux add feature-x`, the agent command is wrapped:

```bash
# Without sandbox:
claude -- "$(cat .workmux/PROMPT-feature-x.md)"

# With sandbox (example for Claude agent):
docker run --rm -it \
  --user 501:20 \
  --mount type=bind,source=/path/to/worktree,target=/path/to/worktree \
  --mount type=bind,source=/path/to/main/.git,target=/path/to/main/.git \
  --mount type=bind,source=/path/to/main,target=/path/to/main \
  --mount type=bind,source=~/.claude-sandbox.json,target=/tmp/.claude.json \
  --mount type=bind,source=~/.claude,target=/tmp/.claude \
  --workdir /path/to/worktree \
  workmux-sandbox:claude \
  sh -c 'claude -- "$(cat .workmux/PROMPT-feature-x.md)"'
```

### What's mounted

| Mount | Access | Purpose |
| --- | --- | --- |
| Worktree directory | read-write | Source code |
| Main worktree | read-write | Symlink resolution (e.g., CLAUDE.md) |
| Main `.git` | read-write | Git operations |
| Agent credentials | read-write | Auth and settings (see [Credentials](./features#credentials)) |
| `extra_mounts` entries | read-only\* | User-configured paths |

\* Extra mounts are read-only by default. Set `writable: true` to allow writes.

For Claude specifically, `~/.claude-sandbox.json` is also mounted to `/tmp/.claude.json` as a separate config file.

### What's NOT accessible

- `~/.ssh/` (SSH keys)
- `~/.aws/` (AWS credentials)
- `~/.config/` (other app configs)
- Other worktrees

### Networking

Docker and Podman handle host resolution differently:

- **Docker Desktop** (macOS/Windows): `host.docker.internal` resolves to the host automatically.
- **Docker Engine** (Linux): workmux automatically adds `--add-host host.docker.internal:host-gateway` so the container can reach the host. This is a no-op on Docker Desktop.
- **Podman**: Uses `host.containers.internal` as the built-in hostname for host access.

If you have a non-standard networking setup (e.g., remote Docker context), override the hostname the guest uses to reach the host RPC server. This setting must be in your global config (`~/.config/workmux/config.yaml`) -- project-level values are ignored for security to prevent RPC traffic redirection:

```yaml
# ~/.config/workmux/config.yaml
sandbox:
  rpc_host: 192.168.1.5
```

By default, containers have unrestricted network access. To restrict outbound connections to only approved domains, configure [network restrictions](./features#network-restrictions). When enabled, all outbound HTTPS is routed through a host-resident proxy that enforces a domain allowlist, and iptables rules inside the container block any direct connections.

### Debugging with `sandbox shell`

Start an interactive shell inside a container for debugging:

```bash
# Start a new container with the same mounts
workmux sandbox shell

# Exec into the currently running container for this worktree
workmux sandbox shell --exec
```

The `--exec` flag attaches to an existing running container instead of starting a new one. This is useful for inspecting the state of a running agent's environment.

## Custom images

To add tools or customize the sandbox environment, export the Dockerfile and modify it:

```bash
workmux sandbox init-dockerfile        # creates Dockerfile.sandbox
vim Dockerfile.sandbox                  # customize
docker build -t my-sandbox -f Dockerfile.sandbox .
```

Then set the image in your config:

```yaml
sandbox:
  enabled: true
  image: my-sandbox
```

## Security: hooks in sandbox

Pre-merge and pre-remove hooks are always skipped for RPC-triggered merges (`--no-verify --no-hooks` is forced by the host). This prevents a compromised guest from injecting malicious hooks via `.workmux.yaml` and triggering them on the host. Similarly, `SpawnAgent` RPC forces `--no-hooks` to skip post-create hooks.