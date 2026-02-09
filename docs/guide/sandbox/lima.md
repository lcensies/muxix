---
description: Run agents in isolated Lima virtual machines
---

# Lima VM backend

workmux can use [Lima](https://lima-vm.io/) VMs for sandboxing, where each project runs in its own virtual machine with a separate kernel.

## Setup

### 1. Install Lima

```bash
brew install lima
```

### 2. Enable in config

```yaml
# ~/.config/workmux/config.yaml or .workmux.yaml
sandbox:
  enabled: true
  backend: lima
```

## Configuration

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

| Option | Default | Description |
| --- | --- | --- |
| `backend` | `container` | Set to `lima` for VM sandboxing |
| `lima.isolation` | `project` | `project` (one VM per repo) or `shared` (single global VM) |
| `lima.projects_dir` | - | Required for `shared` isolation: parent directory of all projects |
| `image` | Debian 12 | Custom qcow2 image URL or `file://` path |
| `lima.skip_default_provision` | `false` | Skip built-in provisioning (system deps + tool install) |
| `lima.cpus` | `4` | Number of CPUs for Lima VMs |
| `lima.memory` | `4GiB` | Memory for Lima VMs |
| `lima.disk` | `100GiB` | Disk size for Lima VMs |
| `lima.provision` | - | Custom user-mode shell script run once at VM creation after built-in steps |
| `toolchain` | `auto` | Toolchain mode: `auto` (detect devbox.json/flake.nix), `off`, `devbox`, or `flake` |
| `host_commands` | `[]` | Commands to proxy from guest to host via RPC (see [shared features](./features#host-command-proxying)) |
| `env_passthrough` | `["GITHUB_TOKEN"]` | Environment variables to pass through to the VM |
| `extra_mounts` | `[]` | Additional host paths to mount (see [shared features](./features#extra-mounts)) |

VM resource and provisioning settings (`isolation`, `projects_dir`, `cpus`, `memory`, `disk`, `provision`, `skip_default_provision`) are nested under `lima`. Settings shared by both backends (`toolchain`, `host_commands`, `env_passthrough`, `image`, `target`) remain at the `sandbox` level. Container-specific settings (`runtime`) are nested under `container`.

## How it works

When using the Lima backend, each sandboxed pane runs a supervisor process (`workmux sandbox run`) that:

1. Ensures the Lima VM is running (creates it on first use)
2. Starts a TCP RPC server on a random port
3. Runs the agent command inside the VM via `limactl shell`
4. Handles RPC requests from the guest workmux binary

The guest VM connects back to the host via `host.lima.internal` (Lima's built-in hostname) to send RPC requests like status updates and agent spawning.

### VM naming scheme

VMs are named deterministically based on the isolation level:

- **Project isolation** (default): `wm-<project>-<hash>` (e.g., `wm-myproject-a1b2c3d4`). The project name (up to 18 characters) is included for readability in `limactl list`.
- **Shared isolation**: `wm-<hash>` (e.g., `wm-5f6g7h8i`). A single global VM is used for all projects.

### Auto-start behavior

VMs are created on first use and started automatically when needed. If a VM already exists but is stopped, workmux restarts it. You don't need to manage VM lifecycle manually during normal use.

### Rosetta on ARM64

On Apple Silicon (ARM64) macOS, workmux automatically enables Rosetta in the Linux VM. This allows x86_64 binaries to run transparently inside the ARM64 VM, providing compatibility with tools that don't have native ARM builds. Rosetta is configured with binfmt support so x86_64 executables are handled automatically.

## Provisioning

### Default provisioning

When a VM is first created, workmux runs two built-in provisioning steps:

**System provision** (as root):
- Installs `curl`, `ca-certificates`, `git`, `xz-utils`

**User provision:**
- Installs the configured agent CLI (based on the `agent` setting)
- Installs [workmux](https://github.com/raine/workmux)
- Installs [Nix](https://nixos.org/) and [Devbox](https://www.jetify.com/devbox) (skipped when `toolchain: off`)

The agent CLI installed depends on your `agent` configuration:

| Agent | What gets installed |
| --- | --- |
| `claude` (default) | Claude Code CLI via `claude.ai/install.sh` |
| `codex` | Codex CLI binary from GitHub releases |
| `gemini` | Node.js + Gemini CLI via npm |
| `opencode` | OpenCode binary via `opencode.ai/install` |

Changing the `agent` setting after VM creation has no effect on existing VMs. Recreate the VM with `workmux sandbox prune` to provision with a different agent.

### Authentication

workmux mounts agent-specific credential directories from the host into the VM. How this works depends on the agent:

| Agent | Host directory | Reuses host auth? |
| --- | --- | --- |
| `claude` | `~/.claude/` | No. Claude stores auth in macOS Keychain, which is not accessible from the VM. You need to authenticate separately inside the VM. |
| `gemini` | `~/.gemini/` | Yes. If you have already authenticated with Gemini on your host, the VM automatically has access to those credentials. |
| `codex` | - | Not mounted |
| `opencode` | - | Not mounted |

For agents with mounted credential directories, authentication done inside the VM is written back to the host directory. This means credentials persist across VM recreations (`workmux sandbox prune`) and you only need to authenticate once.

The credential mount is determined by the `agent` setting at VM creation time. If you switch agents, recreate the VM with `workmux sandbox prune` to get the correct credential mount.

### Custom provisioning

The `provision` field accepts a shell script that runs as a third provisioning step during VM creation, after the built-in steps. Use it to customize the VM environment for your project.

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

- Provisioning only runs when the VM is first created. Changing the `agent` setting or provision script has no effect on existing VMs. Recreate the VM with `workmux sandbox prune` to apply changes.
- With `lima.isolation: shared`, only the first project to create the VM gets its agent installed and provision script run. Use `lima.isolation: project` (default) if different projects use different agents or need different provisioning.
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
- User provision (agent CLI, workmux, Nix/Devbox)

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

## Nix and Devbox toolchain

The Lima backend has built-in support for [Nix](https://nixos.org/) and [Devbox](https://www.jetify.com/devbox) to provide declarative, cached toolchain management inside VMs. For the container backend, use a [custom Dockerfile](./container#custom-images) to install project-specific tools, or use [`host_commands`](./features#host-command-proxying) to proxy commands from the container to the host's toolchain environment.

By default (`toolchain: auto`), workmux checks for `devbox.json` or `flake.nix` in the project root and wraps agent commands in the appropriate environment:

- **Devbox**: Commands run via `devbox run -- <command>`
- **Nix flakes**: Commands run via `nix develop --command bash -c '<command>'`

If both `devbox.json` and `flake.nix` exist, Devbox takes priority.

### Example: Rust project with Devbox

Add a `devbox.json` to your project root:

```json
{
  "packages": ["rustc@latest", "cargo@latest", "just@latest", "ripgrep@latest"]
}
```

When workmux creates a sandbox, the agent automatically has access to `rustc`, `cargo`, `just`, and `rg` without any provisioning scripts.

### Example: Node.js project with Devbox

```json
{
  "packages": ["nodejs@22", "yarn@latest"],
  "shell": {
    "init_hook": ["echo 'Node.js environment ready'"]
  }
}
```

### Disabling toolchain integration

To disable auto-detection (e.g., if your project has a `devbox.json` that should not be used in the sandbox):

```yaml
sandbox:
  backend: lima
  toolchain: off
```

This also skips Nix and Devbox installation during provisioning, making VM creation faster.

To force a specific toolchain mode regardless of which config files exist:

```yaml
sandbox:
  backend: lima
  toolchain: devbox # or: flake
```

### How it works

Nix and Devbox are pre-installed during VM provisioning. Tools declared in `devbox.json` or `flake.nix` are downloaded as pre-built binaries from the Nix binary cache, so no compilation is needed.

The `/nix/store` persists inside the VM across sessions, so subsequent activations are instant. If the VM is pruned with `workmux sandbox prune`, packages will be re-downloaded on next use.

### Toolchain vs provisioning

Use **toolchain** (`devbox.json`/`flake.nix`) for project-specific development tools like compilers, linters, and build tools. Changes take effect on the next sandboxed session without recreating the VM.

Use **provisioning** for one-time VM setup like system packages, shell configuration, or services that need to run as root. Provisioning only runs on VM creation.

## VM management

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

### Stopping VMs

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

