---
description: Run agents in isolated Docker or Podman containers
---

# Container backend

The container sandbox runs agents in isolated Docker or Podman containers, providing lightweight, ephemeral environments that reset after every session.

## Setup

### 1. Install a container runtime

```bash
# macOS
brew install --cask docker          # Docker Desktop
# or
brew install --cask orbstack        # OrbStack (Docker-compatible)
# or
brew install podman                 # Podman
```

### 2. Enable sandbox in config

Add to your global or project config:

```yaml
# ~/.config/workmux/config.yaml or .workmux.yaml
sandbox:
  enabled: true
```

The pre-built image (`ghcr.io/raine/workmux-sandbox:{agent}`) is pulled automatically on first run based on your configured agent. No manual build step is needed, but possible if required (see [custom images](#custom-images)).

To pull the latest image explicitly:

```bash
workmux sandbox pull
```

## Configuration

| Option                    | Default                                 | Description                                                                                                                                                                                     |
| ------------------------- | --------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `enabled`                 | `false`                                 | Enable container sandboxing                                                                                                                                                                     |
| `container.runtime`       | auto-detect                             | Container runtime: `docker` or `podman`. Auto-detected from PATH when not set (prefers docker).                                                                                                 |
| `target`                  | `agent`                                 | Which panes to sandbox: `agent` or `all`                                                                                                                                                        |
| `image`                   | `ghcr.io/raine/workmux-sandbox:{agent}` | Container image name (auto-resolved from configured agent). **Global config only.**                                                                                                             |
| `rpc_host`                | auto                                    | Override hostname for guest-to-host RPC. Defaults to `host.docker.internal` (Docker) or `host.containers.internal` (Podman). Useful for non-standard networking setups. **Global config only.** |
| `env_passthrough`         | `[]`                                    | Environment variables to pass through. **Global config only.**                                                                                                                                  |
| `extra_mounts`            | `[]`                                    | Additional host paths to mount (see [shared features](./features#extra-mounts)). **Global config only.**                                                                                        |
| `network.policy`          | `allow`                                 | Network restriction policy: `allow` (no restrictions) or `deny` (block all except allowed domains). See [network restrictions](#network-restrictions). **Global config only.**        |
| `network.allowed_domains` | `[]`                                    | Allowed outbound HTTPS domains when policy is `deny`. Supports exact matches and `*.` wildcard prefixes. **Global config only.**                                                                |

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

# With sandbox:
docker run --rm -it \
  --user 501:20 \
  --env HOME=/tmp \
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

| Mount                  | Access      | Purpose                                                       |
| ---------------------- | ----------- | ------------------------------------------------------------- |
| Worktree directory     | read-write  | Source code                                                   |
| Main worktree          | read-write  | Symlink resolution (e.g., CLAUDE.md)                          |
| Main `.git`            | read-write  | Git operations                                                |
| Agent credentials      | read-write  | Auth and settings (see [Credentials](./features#credentials)) |
| `extra_mounts` entries | read-only\* | User-configured paths                                         |

\* Extra mounts are read-only by default. Set `writable: true` to allow writes.

For Claude specifically, `~/.claude-sandbox.json` is also mounted to `/tmp/.claude.json` as a separate config file.

### Networking

By default, containers have unrestricted network access. To restrict outbound connections to only approved domains, configure [network restrictions](#network-restrictions). When enabled, all outbound HTTPS is routed through a host-resident proxy that enforces a domain allowlist, and iptables rules inside the container block any direct connections.

### Debugging with `sandbox shell`

Start an interactive shell inside a container for debugging:

```bash
# Start a new container with the same mounts
workmux sandbox shell

# Exec into the currently running container for this worktree
workmux sandbox shell --exec
```

The `--exec` flag attaches to an existing running container instead of starting a new one. This is useful for inspecting the state of a running agent's environment.

## Network restrictions

Network restrictions block outbound connections from sandboxed containers, only allowing traffic to domains you explicitly whitelist. This prevents agents from accessing your local network, exfiltrating data to unauthorized services, or making unintended API calls.

### Configuration

Add to global config (`~/.config/workmux/config.yaml`):

```yaml
sandbox:
  enabled: true
  network:
    policy: deny
    allowed_domains:
      # Claude Code (adjust for your agent)
      - "api.anthropic.com"
      - "platform.claude.com"
```

`network` is a global-only setting. If set in a project's `.workmux.yaml`, it is ignored and a warning is logged. This ensures that project config cannot weaken network restrictions set by the user.

Domain entries support exact matches (`github.com`) and wildcard prefixes (`*.github.com`). Wildcards match subdomains only, not the base domain itself (e.g., `*.github.com` matches `api.github.com` but not `github.com`).

### How it works

Two layers enforce the restrictions:

1. **iptables firewall** inside the container blocks all direct outbound connections, forcing traffic through a host-resident proxy.
2. **CONNECT proxy** on the host checks each domain against the allowlist and rejects connections to private/internal IPs.

This means agents cannot bypass restrictions by ignoring proxy environment variables.

Only HTTPS (port 443) to allowed domains gets through. The proxy also rejects connections to private/internal IP ranges (RFC1918, link-local, loopback), so allowed domains cannot be used to reach local network services. Non-HTTPS protocols like `git+ssh` are blocked; use HTTPS git remotes instead. IPv6 is blocked to prevent bypassing the IPv4 firewall.

### Known limitations

- **Non-HTTP protocols**: Protocols like `git+ssh` are blocked. Use HTTPS git remotes (`git clone https://...`) instead of SSH (`git clone git@...`).
- **Podman rootless**: Network restrictions require `CAP_NET_ADMIN` for iptables. On rootless Podman, this may require additional configuration depending on your setup.

## Custom images

To add tools or customize the sandbox environment, export the Dockerfile and modify it:

```bash
workmux sandbox init-dockerfile        # creates Dockerfile.sandbox
vim Dockerfile.sandbox                 # customize
docker build -t my-sandbox -f Dockerfile.sandbox .
```

To build the default image locally instead of pulling from the registry:

```bash
workmux sandbox build
```

Then set the image in your config:

```yaml
sandbox:
  enabled: true
  image: my-sandbox
```

## Security: hooks in sandbox

Pre-merge and pre-remove hooks are always skipped for RPC-triggered merges (`--no-verify --no-hooks` is forced by the host). This prevents a compromised guest from injecting malicious hooks via `.workmux.yaml` and triggering them on the host. Similarly, `SpawnAgent` RPC forces `--no-hooks` to skip post-create hooks.
