---
description: Run agents in isolated containers or VMs for enhanced security
---

# Sandbox

workmux has first-class support for sandboxing agents inside containers or VMs. Agents are restricted to the project worktree; sensitive files like SSH keys, AWS credentials, and other secrets are not accessible. This lets you run agents in YOLO mode without worrying about what they might touch on your host.

Sandboxing is designed to be transparent. The multiplexer integration should work the same way whether agents are sandboxed or not: status indicators, the dashboard, spawning new agents, and merging all continue to work normally. A built-in RPC protocol bridges the sandbox boundary so that workmux features on the host stay in sync with what agents do inside the sandbox.

<style>
.sandbox-screenshot {
  border-radius: 4px;
  filter: drop-shadow(0 8px 8px rgba(0, 0, 0, 0.5));
}
.dark .sandbox-screenshot {
  filter: drop-shadow(0 8px 8px rgba(0, 200, 220, 0.15));
}
</style>

<div style="margin: 24px 0; padding-bottom: 16px;">
  <img src="/sandbox-claude.webp" alt="Claude Code running inside a Lima VM sandbox" class="sandbox-screenshot">
</div>

## Security model

When sandbox is enabled, agents have access to:

- The current worktree directory (read-write)
- The main worktree directory (read-write, for symlink resolution like `CLAUDE.local.md`)
- The shared `.git` directory (read-write, for git operations)
- Agent settings and credentials (see [credentials](./features#credentials))

Host secrets like SSH keys, AWS credentials, and GPG keys are not accessible. Additional directories can be mounted via [`extra_mounts`](./features#extra-mounts).

Outbound network access can be restricted to only approved domains using [network restrictions](./container#network-restrictions) (container backend). When enabled, a CONNECT proxy and iptables firewall work together to block unauthorized connections and prevent access to internal networks.

## Choosing a backend

workmux supports two sandboxing backends:

|                      | Container (Docker/Podman)                                              | Lima VM                                                          |
| -------------------- | ---------------------------------------------------------------------- | ---------------------------------------------------------------- |
| **Isolation**        | Process-level (namespaces)                                             | Machine-level (virtual machine)                                  |
| **Persistence**      | Ephemeral (new container per session)                                  | Persistent (stateful VMs)                                        |
| **Toolchain**        | Custom Dockerfile or [host commands](./features#host-command-proxying) | Built-in [Nix & Devbox](./lima#nix-and-devbox-toolchain) support |
| **Credential model** | Shared with host (see [credentials](./features#credentials))           | Shared with host (see [credentials](./features#credentials))     |
| **Network**          | Optional [restrictions](./container#network-restrictions) (domain allowlist) | Unrestricted                                                |
| **Platform**         | macOS, Linux                                                           | macOS, Linux                                                     |

Container is a good default: it's simple to set up and ephemeral, so no state accumulates between sessions. Choose Lima if you want persistent VMs with built-in Nix/Devbox toolchain support.

## Adding tools to the sandbox

Agents often need project tooling (compilers, linters, build tools) available inside the sandbox. There are several ways to provide this depending on your backend:

| Approach                   | Container | Lima | Details                                                                                                                              |
| -------------------------- | --------- | ---- | ------------------------------------------------------------------------------------------------------------------------------------ |
| **Host commands**          | Yes       | Yes  | Proxy specific commands to the host via RPC. See [host command proxying](./features#host-command-proxying).                          |
| **Nix / Devbox toolchain** | No        | Yes  | Declare tools in `devbox.json` or `flake.nix` and they're available automatically. See [toolchain](./lima#nix-and-devbox-toolchain). |
| **Custom provisioning**    | No        | Yes  | Run a shell script at VM creation to install packages. See [custom provisioning](./lima#custom-provisioning).                        |
| **Custom Dockerfile**      | Yes       | No   | Build a custom container image with your tools baked in. See [custom images](./container#custom-images).                             |

## Quick start

### Container backend

Install [Docker](https://www.docker.com/) or [Podman](https://podman.io/), then enable in config:

```yaml
# ~/.config/workmux/config.yaml or .workmux.yaml
sandbox:
  enabled: true
```

The pre-built image is pulled automatically on first run. See the [container backend](./container) page for details.

### Lima VM backend

Install [Lima](https://lima-vm.io/) (`brew install lima`), then enable in config:

```yaml
# ~/.config/workmux/config.yaml or .workmux.yaml
sandbox:
  enabled: true
  backend: lima
```

The VM is created and provisioned automatically on first run. See the [Lima VM backend](./lima) page for details.

