---
description: Features shared across container and Lima sandbox backends
---

# Shared features

These features work with both the container and Lima sandbox backends.

## Extra mounts

The `extra_mounts` option lets you mount additional host directories into the sandbox. Mounts are read-only by default for security.

Each entry can be a simple path string (read-only, mirrored into the guest at the same path) or a detailed spec with `host_path`, optional `guest_path`, and optional `writable` flag.

```yaml
sandbox:
  extra_mounts:
    # Simple: read-only, same path in guest
    - ~/notes

    # Detailed: writable with custom guest path
    - host_path: ~/shared-data
      guest_path: /mnt/shared
      writable: true
```

Paths starting with `~` are expanded to the user's home directory. When `guest_path` is omitted, the expanded host path is used as the guest mount point.

**Note:** For the Lima backend, mount changes only take effect when the VM is created. To apply changes to an existing VM, recreate it with `workmux sandbox prune`.

## Host command proxying

The `host_commands` option lets agents inside the sandbox run specific commands on the host machine. It's useful for project toolchain commands (build tools, task runners, linters) that are available on the host via Devbox or Nix but would be slow or complex to install inside the sandbox.

```yaml
# ~/.config/workmux/config.yaml
sandbox:
  host_commands: ["just", "cargo", "npm"]
```

`host_commands` is only read from your global config. If set in a project's `.workmux.yaml`, it is ignored and a warning is logged. This prevents a cloned repository from granting itself host access.

When configured, workmux creates shim scripts inside the sandbox that transparently forward these commands to the host via RPC. The host runs them in the project's toolchain environment (Devbox/Nix), streams stdout/stderr back to the sandbox in real-time, and returns the exit code.

Some commands are built-in and always available as host-exec shims without configuration (e.g., `afplay` for sound notifications). Only commands listed in `host_commands` or built-in are allowed; there is no wildcard or auto-discovery.

For Lima VMs: This is complementary to the toolchain integration (`toolchain: auto`). The toolchain wraps the _agent command_ itself (e.g., `claude`), while `host_commands` lets the agent invoke _other_ tools that exist on the host. For example, an agent running inside the VM could run `just check` and the command would execute on the host with full access to the project's Devbox environment.

### Security model

Host-exec is designed to be secure against a compromised agent inside the sandbox:

- **Command allowlist**: Only commands explicitly listed in `host_commands` (or built-in) can be executed. The allowlist is enforced on the host side.
- **Strict command names**: Command names must match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`. No path separators, shell metacharacters, or special names (`.`, `..`) are accepted.
- **No shell injection**: When toolchain wrapping is active (devbox/nix), command arguments are passed as positional parameters to bash (`"$@"`), never interpolated into a shell string. Without toolchain wrapping, commands are executed directly via the OS with no shell involved.
- **Environment isolation**: Child processes run with a sanitized environment. Only essential variables (`PATH`, `HOME`, `TERM`, etc.) are passed through. Host secrets like API keys are not inherited. `PATH` is normalized to absolute entries only to prevent relative-path hijacking.
- **Filesystem sandbox**: On macOS, child processes run under `sandbox-exec` (Seatbelt), which denies access to sensitive directories (`~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.kube`, `~/.docker`, keychains, browser data) and denies writes to `$HOME` except toolchain caches (`.cache`, `.cargo`, `.rustup`, `.npm`). On Linux, `bwrap` (Bubblewrap) provides similar isolation with a read-only root filesystem, tmpfs over secret directories, and a writable worktree bind mount. If `bwrap` is not installed on Linux, commands run without filesystem sandboxing (with a warning).
- **Global-only allowlist**: `host_commands` is only read from global config (`~/.config/workmux/config.yaml`). Project-level `.workmux.yaml` cannot set it. A warning is logged if it tries.
- **RPC authentication**: Each session uses a random 256-bit token. Requests exceeding 1MB are rejected to prevent memory exhaustion.
- **Worktree-locked**: All commands execute with the project worktree as the working directory.

**Known limitations**:

- Allowlisted commands that read project files (build tools like `just`, `cargo`, `make`) effectively act as code interpreters. A compromised agent can write a malicious `justfile` and then invoke `just`. The filesystem sandbox mitigates this by blocking access to host secrets and restricting writes, but the child process still has network access (required for package managers).
- `sandbox-exec` is deprecated on macOS but remains functional. Apple has not announced a replacement for CLI tools.
- On Linux, `bwrap` must be installed separately (`apt install bubblewrap`). Without it, only environment sanitization is applied.

## Sound notifications

Claude Code hooks often use `afplay` to play notification sounds (e.g., when an agent finishes). Since `afplay` is a macOS-only binary, it doesn't exist inside the Linux guest. workmux includes `afplay` as a built-in host-exec shim that forwards sound playback to the host. This works with both Lima and container backends.

This is transparent: when a hook runs `afplay /System/Library/Sounds/Glass.aiff` inside the sandbox, the shim runs `afplay` on the host via the host-exec RPC mechanism. No configuration is needed.

## Credentials

The container and Lima backends handle credentials differently:

**Container backend:** Uses separate credentials stored in `~/.claude-sandbox.json` on the host. Agents authenticate interactively on first use inside the container. The host `~/.claude/` directory is mounted for settings (project configs, MCP servers, etc.).

**Lima backend:** Mounts the host's `~/.claude/` directory into the guest VM at `$HOME/.claude/`. This means the VM shares your host credentials, so no separate auth step is needed. When you authenticate Claude Code on the host, the VM picks it up automatically, and vice versa.

The Lima backend also seeds a minimal `~/.claude.json` with onboarding marked as complete, so agents don't trigger the onboarding flow on every VM creation. This is stored per-VM in `~/.local/state/workmux/lima/<vm-name>/` and symlinked into the guest. These state directories are cleaned up automatically by `workmux sandbox prune`.

| | Container | Lima |
| --- | --- | --- |
| Credential storage | `~/.claude-sandbox.json` (separate) | `~/.claude/.credentials.json` (shared with host) |
| Settings directory | `~/.claude/` (shared with host) | `~/.claude/` (shared with host) |
| Auth setup | Agent authenticates on first use | None needed |

## RPC protocol

The supervisor and guest communicate via JSON-lines over TCP. Each request is a single JSON object on one line.

**Supported requests:**

- `SetStatus` - updates the tmux pane status icon (working/waiting/done/clear)
- `SetTitle` - renames the tmux window
- `Heartbeat` - health check, returns Ok
- `SpawnAgent` - runs `workmux add` on the host to create a new worktree and pane
- `Exec` - runs a command on the host and streams stdout/stderr back (used by host-exec shims, including built-in `afplay`)
- `Merge` - runs `workmux merge` on the host with all flags forwarded

Requests are authenticated with a per-session token passed via the `WM_RPC_TOKEN` environment variable.

## Troubleshooting

### Agent can't find credentials (container)

Container agents authenticate interactively on first use. If credentials are missing, start a shell in the container with `workmux sandbox shell` and run the agent to trigger authentication.

## Installing local builds

During development, the macOS host binary cannot run inside Linux containers or VMs. Use `install-dev` to cross-compile and install your local workmux build:

```bash
# First time: install prerequisites
rustup target add aarch64-unknown-linux-gnu
brew install messense/macos-cross-toolchains/aarch64-unknown-linux-gnu

# Cross-compile and install into containers and running VMs
workmux sandbox install-dev

# After code changes, rebuild and reinstall
workmux sandbox install-dev

# Use --release for optimized builds
workmux sandbox install-dev --release

# Skip rebuild if binary hasn't changed
workmux sandbox install-dev --skip-build
```

For containers, this builds a thin overlay image (`FROM <image>` + `COPY workmux`) on top of the configured sandbox image, replacing it in-place. For Lima VMs, the binary is installed to `~/.local/bin/workmux` inside each running VM.
