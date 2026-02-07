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

### 2. Build the sandbox image

```bash
workmux sandbox build
```

This builds a Docker image named `workmux-sandbox` containing:

- Claude Code CLI
- workmux (installed from GitHub releases)
- Nix and Devbox
- Git and other dependencies

### Custom images

To add tools or customize the sandbox environment, export the Dockerfile
template and modify it:

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

This saves credentials to `~/.claude-sandbox.json`, which is mounted into containers. The host `~/.claude/` directory is also mounted for settings.

## Configuration

| Option            | Default            | Description                              |
| ----------------- | ------------------ | ---------------------------------------- |
| `enabled`         | `false`            | Enable container sandboxing              |
| `runtime`         | `docker`           | Container runtime: `docker` or `podman`  |
| `target`          | `agent`            | Which panes to sandbox: `agent` or `all` |
| `image`           | `workmux-sandbox`  | Container image name                     |
| `env_passthrough` | `["GITHUB_TOKEN"]` | Environment variables to pass through    |

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
  --mount type=bind,source=/path/to/main,target=/path/to/main \
  --mount type=bind,source=~/.claude-sandbox.json,target=/tmp/.claude.json \
  --mount type=bind,source=~/.claude,target=/tmp/.claude \
  --workdir /path/to/worktree \
  workmux-sandbox \
  sh -c 'claude -- "$(cat .workmux/PROMPT-feature-x.md)"'
```

### What's mounted

| Mount                    | Access     | Purpose                              |
| ------------------------ | ---------- | ------------------------------------ |
| Worktree directory       | read-write | Source code                          |
| Main worktree            | read-write | Symlink resolution (e.g., CLAUDE.md) |
| Main `.git`              | read-write | Git operations                       |
| `~/.claude-sandbox.json` | read-write | Agent config                         |
| `~/.claude/`             | read-write | Agent settings                       |

### What's NOT accessible

- `~/.ssh/` (SSH keys)
- `~/.aws/` (AWS credentials)
- `~/.config/` (other app configs)
- Other worktrees

## Limitations

### Coordinator agents

If a coordinator agent spawns sub-agents via workmux, those sub-agents run outside the sandbox on the host. This is a fundamental limitation of the architecture. For fully sandboxed coordination, run coordinators on the host and only sandbox leaf agents.

### `workmux merge` must run on host

The `merge` command requires access to multiple worktrees, which breaks the sandbox isolation model. Always run `workmux merge` from outside the sandbox (on the host terminal).

### macOS tmux bridge

On macOS with Docker Desktop, status updates require a TCP bridge because Unix sockets don't work across the VM boundary. This is optional for basic functionality.

## Troubleshooting

### Git commands fail with "not a git repository"

The main `.git` directory must be mounted. Check that your worktree has a valid `.git` file pointing to the main repository.

### Permission denied on files

The container runs as your host user (UID:GID). Ensure your image doesn't require root permissions for the agent.

### Agent can't find credentials

Run `workmux sandbox auth` to authenticate inside the container. Credentials are separate from host credentials.

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

```yaml
sandbox:
  enabled: true
  backend: lima
  isolation: project # default: one VM per git repository
  cpus: 8
  memory: 8GiB
  env_passthrough:
    - GITHUB_TOKEN
    - ANTHROPIC_API_KEY
  provision: |
    sudo apt-get install -y ripgrep fd-find jq
```

| Option                   | Default            | Description                                                                          |
| ------------------------ | ------------------ | ------------------------------------------------------------------------------------ |
| `backend`                | `container`        | Set to `lima` for VM sandboxing                                                      |
| `isolation`              | `project`          | `project` (one VM per repo) or `user` (single global VM)                             |
| `projects_dir`           | -                  | Required for `user` isolation: parent directory of all projects                      |
| `image`                  | Debian 12          | Custom qcow2 image URL or `file://` path                                             |
| `skip_default_provision` | `false`            | Skip built-in provisioning (system deps + tool install)                              |
| `cpus`                   | `4`                | Number of CPUs for Lima VMs                                                          |
| `memory`                 | `4GiB`             | Memory for Lima VMs                                                                  |
| `disk`                   | `100GiB`           | Disk size for Lima VMs                                                               |
| `provision`              | -                  | Custom user-mode shell script run once at VM creation after built-in steps           |
| `toolchain`              | `auto`             | Toolchain mode: `auto` (detect devbox.json/flake.nix), `off`, `devbox`, or `flake`  |
| `host_commands`          | `[]`               | Commands to proxy from guest to host via RPC (e.g., `["just", "cargo"]`)             |
| `env_passthrough`        | `["GITHUB_TOKEN"]` | Environment variables to pass through to the VM                                      |

### Host command proxying

The `host_commands` option lets agents inside the sandbox run specific commands on the host machine. This works with both Lima and container backends. It's useful for project toolchain commands (build tools, task runners, linters) that are available on the host via Devbox or Nix but would be slow or complex to install inside the sandbox.

```yaml
sandbox:
  host_commands: ["just", "cargo", "npm"]
```

When configured, workmux creates shim scripts inside the sandbox that transparently forward these commands to the host via RPC. The host runs them in the project's toolchain environment (Devbox/Nix), streams stdout/stderr back to the sandbox in real-time, and returns the exit code.

Only commands explicitly listed in `host_commands` are allowed -- there is no wildcard or auto-discovery. Commands containing path separators are rejected, and execution is locked to the project's worktree directory.

For Lima VMs: This is complementary to the toolchain integration (`toolchain: auto`). The toolchain wraps the *agent command* itself (e.g., `claude`), while `host_commands` lets the agent invoke *other* tools that exist on the host. For example, an agent running inside the VM could run `just check` and the command would execute on the host with full access to the project's Devbox environment.

### Custom provisioning

The `provision` field accepts a shell script that runs as a third provisioning step during VM creation, after the built-in steps that install core dependencies (git, curl, Claude CLI, workmux). Use it to customize the VM environment for your project.

The script runs in `user` mode. Use `sudo` for system-level commands.

```yaml
sandbox:
  backend: lima
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
- With `isolation: user` (shared VM), only the first project to create the VM gets its provision script run. Use `isolation: project` (default) if different projects need different provisioning.
- The built-in system step runs `apt-get update` before the custom script, so package lists are already available.

### Custom images

You can use a pre-built qcow2 image to skip provisioning entirely, reducing VM creation time from minutes to seconds. This is useful when you want every VM to start from an identical, known-good state.

```yaml
sandbox:
  backend: lima
  image: file:///Users/me/.lima/images/workmux-golden.qcow2
  skip_default_provision: true
```

When `image` is set, it replaces the default Debian 12 genericcloud image. The value can be a `file://` path to a local qcow2 image or an HTTP(S) URL.

When `skip_default_provision` is true, the built-in provisioning steps are skipped:

- System provision (apt-get install of curl, ca-certificates, git)
- User provision (Claude CLI, workmux, afplay shim)

Custom `provision` scripts still run even when `skip_default_provision` is true, so you can layer additional setup on top of a pre-built image.

#### Creating a pre-built image

1. Create a VM with default provisioning and let it fully provision:

   ```yaml
   sandbox:
     backend: lima
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
     skip_default_provision: true
   ```

New VMs will now boot from the snapshot with everything pre-installed.

### Nix and Devbox toolchain

The Lima backend has built-in support for [Nix](https://nixos.org/) and [Devbox](https://www.jetify.com/devbox) to provide declarative, cached toolchain management inside VMs.

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
  toolchain: devbox  # or: flake
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
- `Notify` -- triggers host-side notifications (e.g., playing sounds via `afplay`)
- `Exec` -- runs a command on the host and streams stdout/stderr back (used by `host_commands` shims)

Requests are authenticated with a per-session token passed via the `WM_RPC_TOKEN` environment variable.

### Sound notifications

Claude Code hooks often use `afplay` to play notification sounds (e.g., when an agent finishes). Since `afplay` is a macOS-only binary, it doesn't exist inside the Linux guest VM. workmux solves this by installing an `afplay` shim in the guest that forwards sound playback to the host via RPC.

This is transparent -- when a hook runs `afplay /System/Library/Sounds/Glass.aiff` inside the VM, the shim sends a `Notify` RPC request and the host plays the sound. No configuration is needed.

### Credentials

The container and Lima backends handle credentials differently:

**Container backend:** Uses separate credentials stored in `~/.claude-sandbox.json` on the host. Run `workmux sandbox auth` once to authenticate inside a container. The host `~/.claude/` directory is mounted for settings (project configs, MCP servers, etc.).

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
| Auth setup         | `workmux sandbox auth` required     | None needed                                      |

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
