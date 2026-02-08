---
description: Run agents in isolated Docker or Podman containers for enhanced security
---

# Container Sandbox

The container sandbox runs agents in isolated Docker or Podman containers, restricting their access to only the current worktree. This protects sensitive files like SSH keys, AWS credentials, and other secrets from agent access.

## Security Model

When sandbox is enabled:

- Agents can only access the current worktree directory
- The main `.git` directory is mounted read-write (for git operations)
- Sandbox uses separate authentication stored in `~/.claude-sandbox.json`, with host `~/.claude/` mounted for settings
- Host credentials (SSH keys, AWS, etc.) are not accessible

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

### Custom images

To add tools or customize the sandbox environment, export the Dockerfile
and modify it:

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

## Configuration

| Option              | Default                                 | Description                                                            |
| ------------------- | --------------------------------------- | ---------------------------------------------------------------------- |
| `enabled`           | `false`                                 | Enable container sandboxing                                            |
| `container.runtime` | `docker`                                | Container runtime: `docker` or `podman`                                |
| `target`            | `agent`                                 | Which panes to sandbox: `agent` or `all`                               |
| `image`             | `ghcr.io/raine/workmux-sandbox:{agent}` | Container image name (auto-resolved from configured agent)             |
| `env_passthrough`   | `["GITHUB_TOKEN"]`                      | Environment variables to pass through                                  |
| `extra_mounts`      | `[]`                                    | Additional host paths to mount into the sandbox (read-only by default) |

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
  --mount type=bind,source=/path/to/main,target=/path/to/main \
  --mount type=bind,source=~/.claude-sandbox.json,target=/tmp/.claude.json \
  --mount type=bind,source=~/.claude,target=/tmp/.claude \
  --workdir /path/to/worktree \
  workmux-sandbox \
  sh -c 'claude -- "$(cat .workmux/PROMPT-feature-x.md)"'
```

### What's mounted

| Mount                    | Access      | Purpose                              |
| ------------------------ | ----------- | ------------------------------------ |
| Worktree directory       | read-write  | Source code                          |
| Main worktree            | read-write  | Symlink resolution (e.g., CLAUDE.md) |
| Main `.git`              | read-write  | Git operations                       |
| `~/.claude-sandbox.json` | read-write  | Agent config                         |
| `~/.claude/`             | read-write  | Agent settings                       |
| `extra_mounts` entries   | read-only\* | User-configured paths                |

\* Extra mounts are read-only by default. Set `writable: true` to allow writes.

### What's NOT accessible

- `~/.ssh/` (SSH keys)
- `~/.aws/` (AWS credentials)
- `~/.config/` (other app configs)
- Other worktrees

## Limitations

### Coordinator agents

If a coordinator agent spawns sub-agents via workmux, those sub-agents run outside the sandbox on the host. This is a fundamental limitation of the architecture. For fully sandboxed coordination, run coordinators on the host and only sandbox leaf agents.

### `workmux merge` in sandbox

The `merge` command routes through RPC when run inside a sandbox. The host
supervisor executes the full merge workflow, including tmux cleanup and worktree
deletion. All merge flags are forwarded to the host. Output is streamed back in
real-time so the agent sees merge progress.

**Security:** Pre-merge and pre-remove hooks are always skipped for RPC-triggered
merges (`--no-verify --no-hooks` is forced by the host). This prevents a
compromised guest from injecting malicious hooks via `.workmux.yaml` and
triggering them on the host. Similarly, `SpawnAgent` RPC forces `--no-hooks` to
skip post-create hooks.

### macOS tmux bridge

On macOS with Docker Desktop, status updates require a TCP bridge because Unix sockets don't work across the VM boundary. This is optional for basic functionality.

## Troubleshooting

### Git commands fail with "not a git repository"

The main `.git` directory must be mounted. Check that your worktree has a valid `.git` file pointing to the main repository.

### Permission denied on files

The container runs as your host user (UID:GID). Ensure your image doesn't require root permissions for the agent.

### Agent can't find credentials

Agents authenticate interactively on first use inside the container. If credentials are missing, start a shell in the container with `workmux sandbox shell` and run the agent to trigger authentication.

## Lima VM Backend

workmux can use [Lima](https://lima-vm.io/) VMs for sandboxing on macOS, providing stronger isolation than containers with full VM-level separation.

### How it works

When using the Lima backend, each sandboxed pane runs a supervisor process (`workmux sandbox run`) that:

1. Ensures the Lima VM is running (creates it on first use)
2. Starts a TCP RPC server on a random port
3. Runs the agent command inside the VM via `limactl shell`
4. Handles RPC requests from the guest workmux binary

The guest VM connects back to the host via `host.lima.internal` (Lima's built-in hostname) to send RPC requests like status updates and agent spawning.

### Lima configuration

Lima-specific settings are nested under `sandbox.lima`:

```yaml
sandbox:
  enabled: true
  backend: lima
  env_passthrough:
    - GITHUB_TOKEN
    - ANTHROPIC_API_KEY
  lima:
    isolation: project # default: one VM per git repository
    cpus: 8
    memory: 8GiB
    provision: |
      sudo apt-get install -y ripgrep fd-find jq
```

| Option                        | Default            | Description                                                                        |
| ----------------------------- | ------------------ | ---------------------------------------------------------------------------------- |
| `backend`                     | `container`        | Set to `lima` for VM sandboxing                                                    |
| `lima.isolation`              | `project`          | `project` (one VM per repo) or `shared` (single global VM)                         |
| `lima.projects_dir`           | -                  | Required for `shared` isolation: parent directory of all projects                  |
| `image`                       | Debian 12          | Custom qcow2 image URL or `file://` path                                           |
| `lima.skip_default_provision` | `false`            | Skip built-in provisioning (system deps + tool install)                            |
| `lima.cpus`                   | `4`                | Number of CPUs for Lima VMs                                                        |
| `lima.memory`                 | `4GiB`             | Memory for Lima VMs                                                                |
| `lima.disk`                   | `100GiB`           | Disk size for Lima VMs                                                             |
| `lima.provision`              | -                  | Custom user-mode shell script run once at VM creation after built-in steps         |
| `toolchain`                   | `auto`             | Toolchain mode: `auto` (detect devbox.json/flake.nix), `off`, `devbox`, or `flake` |
| `host_commands`               | `[]`               | Commands to proxy from guest to host via RPC (e.g., `["just", "cargo"]`)           |
| `env_passthrough`             | `["GITHUB_TOKEN"]` | Environment variables to pass through to the VM                                    |
| `extra_mounts`                | `[]`               | Additional host paths to mount into the sandbox (read-only by default)             |

VM resource and provisioning settings (`isolation`, `projects_dir`, `cpus`, `memory`, `disk`, `provision`, `skip_default_provision`) are nested under `lima`. Settings shared by both backends (`toolchain`, `host_commands`, `env_passthrough`, `image`, `target`) remain at the `sandbox` level. Container-specific settings (`runtime`) are nested under `container`.

### Extra mount points

The `extra_mounts` option lets you mount additional host directories into the sandbox. This works with both container and Lima backends. Mounts are read-only by default for security.

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

### Host command proxying

The `host_commands` option lets agents inside the sandbox run specific commands on the host machine. This works with both Lima and container backends. It's useful for project toolchain commands (build tools, task runners, linters) that are available on the host via Devbox or Nix but would be slow or complex to install inside the sandbox.

```yaml
# ~/.config/workmux/config.yaml
sandbox:
  host_commands: ["just", "cargo", "npm"]
```

`host_commands` is only read from your global config. If set in a project's `.workmux.yaml`, it is ignored and a warning is logged. This prevents a cloned repository from granting itself host access.

When configured, workmux creates shim scripts inside the sandbox that transparently forward these commands to the host via RPC. The host runs them in the project's toolchain environment (Devbox/Nix), streams stdout/stderr back to the sandbox in real-time, and returns the exit code.

Some commands are built-in and always available as host-exec shims without configuration (e.g., `afplay` for sound notifications). Only commands listed in `host_commands` or built-in are allowed -- there is no wildcard or auto-discovery.

For Lima VMs: This is complementary to the toolchain integration (`toolchain: auto`). The toolchain wraps the _agent command_ itself (e.g., `claude`), while `host_commands` lets the agent invoke _other_ tools that exist on the host. For example, an agent running inside the VM could run `just check` and the command would execute on the host with full access to the project's Devbox environment.

#### Security model

Host-exec is designed to be secure against a compromised agent inside the sandbox:

- **Command allowlist**: Only commands explicitly listed in `host_commands` (or built-in) can be executed. The allowlist is enforced on the host side.
- **Strict command names**: Command names must match `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`. No path separators, shell metacharacters, or special names (`.`, `..`) are accepted.
- **No shell injection**: When toolchain wrapping is active (devbox/nix), command arguments are passed as positional parameters to bash (`"$@"`), never interpolated into a shell string. Without toolchain wrapping, commands are executed directly via the OS with no shell involved.
- **Environment isolation**: Child processes run with a sanitized environment. Only essential variables (`PATH`, `HOME`, `TERM`, etc.) are passed through -- host secrets like API keys are not inherited. `PATH` is normalized to absolute entries only to prevent relative-path hijacking.
- **Filesystem sandbox**: On macOS, child processes run under `sandbox-exec` (Seatbelt), which denies access to sensitive directories (`~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.kube`, `~/.docker`, keychains, browser data) and denies writes to `$HOME` except toolchain caches (`.cache`, `.cargo`, `.rustup`, `.npm`). On Linux, `bwrap` (Bubblewrap) provides similar isolation with a read-only root filesystem, tmpfs over secret directories, and a writable worktree bind mount. If `bwrap` is not installed on Linux, commands run without filesystem sandboxing (with a warning).
- **Global-only allowlist**: `host_commands` is only read from global config (`~/.config/workmux/config.yaml`). Project-level `.workmux.yaml` cannot set it -- a warning is logged if it tries.
- **RPC authentication**: Each session uses a random 256-bit token. Requests exceeding 1MB are rejected to prevent memory exhaustion.
- **Worktree-locked**: All commands execute with the project worktree as the working directory.

**Known limitations**:

- Allowlisted commands that read project files (build tools like `just`, `cargo`, `make`) effectively act as code interpreters. A compromised agent can write a malicious `justfile` and then invoke `just`. The filesystem sandbox mitigates this by blocking access to host secrets and restricting writes, but the child process still has network access (required for package managers).
- `sandbox-exec` is deprecated on macOS but remains functional. Apple has not announced a replacement for CLI tools.
- On Linux, `bwrap` must be installed separately (`apt install bubblewrap`). Without it, only environment sanitization is applied.

### Custom provisioning

The `provision` field accepts a shell script that runs as a third provisioning step during VM creation, after the built-in steps that install core dependencies (git, curl, Claude CLI, workmux). Use it to customize the VM environment for your project.

The script runs in `user` mode. Use `sudo` for system-level commands.

```yaml
sandbox:
  backend: lima
  lima:
    provision: |
      # Install extra CLI tools
      sudo apt-get install -y ripgrep fd-find jq

      # Install Node.js via nvm
      curl -o- https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.3/install.sh | bash
      export NVM_DIR="$HOME/.nvm"
      . "$NVM_DIR/nvm.sh"
      nvm install 22
```

**Important:**

- Provisioning only runs when the VM is first created. Changing the script has no effect on existing VMs. Recreate the VM with `workmux sandbox prune` to apply changes.
- With `lima.isolation: shared`, only the first project to create the VM gets its provision script run. Use `lima.isolation: project` (default) if different projects need different provisioning.
- The built-in system step runs `apt-get update` before the custom script, so package lists are already available.

### Custom images

You can use a pre-built qcow2 image to skip provisioning entirely, reducing VM creation time from minutes to seconds. This is useful when you want every VM to start from an identical, known-good state.

```yaml
sandbox:
  backend: lima
  image: file:///Users/me/.lima/images/workmux-golden.qcow2
  lima:
    skip_default_provision: true
```

When `image` is set, it replaces the default Debian 12 genericcloud image. The value can be a `file://` path to a local qcow2 image or an HTTP(S) URL.

When `skip_default_provision` is true, the built-in provisioning steps are skipped:

- System provision (apt-get install of curl, ca-certificates, git)
- User provision (Claude CLI, workmux, Nix/Devbox)

Custom `provision` scripts still run even when `skip_default_provision` is true, so you can layer additional setup on top of a pre-built image.

#### Creating a pre-built image

1. Create a VM with default provisioning and let it fully provision:

   ```yaml
   sandbox:
     backend: lima
     lima:
       provision: |
         sudo apt-get install -y ripgrep fd-find jq
   ```

2. After the VM is running, stop it:

   ```bash
   limactl stop wm-yourproject-abc12345
   ```

3. Export the disk image (flattens base + changes into a single file):

   ```bash
   mkdir -p ~/.lima/images
   qemu-img convert -O qcow2 \
     ~/.lima/wm-yourproject-abc12345/diffdisk \
     ~/.lima/images/workmux-golden.qcow2
   ```

4. Update your config to use the pre-built image:

   ```yaml
   sandbox:
     backend: lima
     image: file:///Users/me/.lima/images/workmux-golden.qcow2
     lima:
       skip_default_provision: true
   ```

New VMs will now boot from the snapshot with everything pre-installed.

### Nix and Devbox toolchain

The Lima backend has built-in support for [Nix](https://nixos.org/) and [Devbox](https://www.jetify.com/devbox) to provide declarative, cached toolchain management inside VMs. For the container backend, use a [custom Dockerfile](#custom-images) to install project-specific tools, or use [`host_commands`](#host-command-proxying) to proxy commands from the container to the host's toolchain environment.

By default (`toolchain: auto`), workmux checks for `devbox.json` or `flake.nix` in the project root and wraps agent commands in the appropriate environment:

- **Devbox**: Commands run via `devbox run -- <command>`
- **Nix flakes**: Commands run via `nix develop --command bash -c '<command>'`

If both `devbox.json` and `flake.nix` exist, Devbox takes priority.

#### Example: Rust project with Devbox

Add a `devbox.json` to your project root:

```json
{
  "packages": ["rustc@latest", "cargo@latest", "just@latest", "ripgrep@latest"]
}
```

When workmux creates a sandbox, the agent automatically has access to `rustc`, `cargo`, `just`, and `rg` without any provisioning scripts.

#### Example: Node.js project with Devbox

```json
{
  "packages": ["nodejs@22", "yarn@latest"],
  "shell": {
    "init_hook": ["echo 'Node.js environment ready'"]
  }
}
```

#### Disabling toolchain integration

To disable auto-detection (e.g., if your project has a `devbox.json` that should not be used in the sandbox):

```yaml
sandbox:
  backend: lima
  toolchain: off
```

To force a specific toolchain mode regardless of which config files exist:

```yaml
sandbox:
  backend: lima
  toolchain: devbox # or: flake
```

#### How it works

Nix and Devbox are pre-installed during VM provisioning. Tools declared in `devbox.json` or `flake.nix` are downloaded as pre-built binaries from the Nix binary cache -- no compilation needed.

The `/nix/store` persists inside the VM across sessions, so subsequent activations are instant. If the VM is pruned with `workmux sandbox prune`, packages will be re-downloaded on next use.

#### Toolchain vs provisioning

Use **toolchain** (`devbox.json`/`flake.nix`) for project-specific development tools like compilers, linters, and build tools. Changes take effect immediately without recreating the VM.

Use **provisioning** for one-time VM setup like system packages, shell configuration, or services that need to run as root. Provisioning only runs on VM creation.

### RPC protocol

The supervisor and guest communicate via JSON-lines over TCP. Each request is a single JSON object on one line.

**Supported requests:**

- `SetStatus` -- updates the tmux pane status icon (working/waiting/done/clear)
- `SetTitle` -- renames the tmux window
- `Heartbeat` -- health check, returns Ok
- `SpawnAgent` -- runs `workmux add` on the host to create a new worktree and pane
- `Exec` -- runs a command on the host and streams stdout/stderr back (used by host-exec shims, including built-in `afplay`)
- `Merge` -- runs `workmux merge` on the host with all flags forwarded

Requests are authenticated with a per-session token passed via the `WM_RPC_TOKEN` environment variable.

### Sound notifications

Claude Code hooks often use `afplay` to play notification sounds (e.g., when an agent finishes). Since `afplay` is a macOS-only binary, it doesn't exist inside the Linux guest. workmux includes `afplay` as a built-in host-exec shim that forwards sound playback to the host. This works with both Lima and container backends.

This is transparent -- when a hook runs `afplay /System/Library/Sounds/Glass.aiff` inside the sandbox, the shim runs `afplay` on the host via the host-exec RPC mechanism. No configuration is needed.

### Credentials

The container and Lima backends handle credentials differently:

**Container backend:** Uses separate credentials stored in `~/.claude-sandbox.json` on the host. Agents authenticate interactively on first use inside the container. The host `~/.claude/` directory is mounted for settings (project configs, MCP servers, etc.).

**Lima backend:** Mounts the host's `~/.claude/` directory into the guest VM at `$HOME/.claude/`. This means the VM shares your host credentials -- no separate auth step is needed. When you authenticate Claude Code on the host, the VM picks it up automatically, and vice versa.

The Lima backend also seeds a minimal `~/.claude.json` with onboarding marked as
complete, so agents don't trigger the onboarding flow on every VM creation. This
is stored per-VM in `~/.local/state/workmux/lima/<vm-name>/` and symlinked into
the guest. These state directories are cleaned up automatically by
`workmux sandbox prune`.

|                    | Container                           | Lima                                             |
| ------------------ | ----------------------------------- | ------------------------------------------------ |
| Credential storage | `~/.claude-sandbox.json` (separate) | `~/.claude/.credentials.json` (shared with host) |
| Settings directory | `~/.claude/` (shared with host)     | `~/.claude/` (shared with host)                  |
| Auth setup         | Agent authenticates on first use    | None needed                                      |

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
   Age: 2 hours ago
   Last accessed: 5 minutes ago

2. wm-another-proj-d1370a2a (Stopped)
   Age: 1 day ago
   Last accessed: 1 day ago

Delete all these VMs? [y/N]
```

Lima VMs are stored in `~/.lima/<name>/`.

### Installing local builds into sandboxes

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
