# wng — Extended Features over Muxix

This document covers features **added in wng that do not exist in upstream muxix**.
Upstream provides: worktree lifecycle (add/open/close/merge/remove), sidebar, dashboard (Agents + Worktrees tabs), sandbox (Lima/container), status tracking, multiplexer support (tmux/wezterm/kitty/zellij).

---

## Extended Dashboard TUI — Tasks Tab

Upstream has two dashboard tabs: **Agents** and **Worktrees**. wng adds **Tasks**,
a view over the project's task graph (`tasks/index.json`).

```bash
muxix dashboard              # opens on Agents tab
muxix dashboard -t tasks     # open directly on Tasks
```

### Tasks Tab

Live view of `tasks/index.json`. Shows status (○ todo / ◐ in_progress / ● done / ✗ failed), task ID, title, dependencies, and assigned worktree. Supports inline CRUD — create, edit, delete tasks without leaving the TUI.

Key bindings (Tasks tab):
- `n` — new task (opens form)
- `e` — edit selected task
- `d` — delete selected task (confirms)
- `/` — filter tasks
- `Enter` — open task detail
- `Tab` — cycle tabs

## Task Management

Three interfaces for `tasks/index.json` — CLI, TUI, and JSON HTTP API.

### CLI (`muxix task`)

```bash
# List all tasks
muxix task list

# Filter by status
muxix task list --status todo

# Show only frontier (ready to start)
muxix task list --frontier

# JSON output
muxix task list --json

# Get a single task
muxix task get auth-module

# Create a task
muxix task create \
  --id auth-module \
  --title "Implement auth" \
  --description "JWT-based auth with refresh tokens" \
  --depends-on ""  # comma-separated IDs

# Update fields
muxix task update auth-module --status in_progress
muxix task update auth-module --title "Auth module (revised)"

# Delete
muxix task delete auth-module
```

Relative `--graph` paths resolve to the **main worktree root**, so agents running inside a feature worktree automatically share the project's single graph.

## Signal System

Out-of-band agent signaling — an external harness reads these instead of parsing agent output.

```bash
muxix signal <kind> [--pane <pane-id>] [--node <node-id>] [--feedback "..."]
```

### Pane-keyed signals (agent lifecycle hooks)

Keyed by `$TMUX_PANE` (or `--pane`). Written to `.muxix/signals/<pane-id>/<kind>`.

| Kind | Trigger | Effect |
|---|---|---|
| `turn-done` | Stop hook | Agent finished a response turn |
| `needs-input` | Notification hook | Agent blocked waiting for user |
| `working` | PostToolUse / UserPromptSubmit | Agent resumed; clears `needs-input` |
| `proceed` | `/implement` slash command | Release current breakpoint gate |
| `reject` | `/reject` slash command | Release gate with rejection + feedback |
| `session-ready` | SessionStart hook | Agent session initialized |

Usage in Claude Code hooks:
```bash
# ~/.claude/settings.json hooks:
{
  "hooks": {
    "Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "muxix signal turn-done"}]}],
    "Notification": [{"matcher": "", "hooks": [{"type": "command", "command": "muxix signal needs-input"}]}]
  }
}
```

### Node-keyed signals (inter-stage messaging)

Keyed by `--node <node-id>`. For agents to signal stage completion to whatever harness is driving them.

| Kind | When | Effect |
|---|---|---|
| `done` | Agent finishes stage | Advances runner to next node |
| `error` | Agent encounters error | Triggers `on_failure` handling |

```bash
# At end of agent prompt:
muxix signal done --node implement

# On failure:
muxix signal error --node implement --feedback "Tests failed: 3 assertions"
```

---

## Agent Profile System

Restructured from a flat `agent_setup/` + `agent_identity.rs` into a proper `agent/` module with a trait-based profile system.

### `AgentProfile` trait (`src/agent/profile.rs`)

```rust
pub trait AgentProfile: Send + Sync {
    fn name(&self) -> &'static str;
    fn needs_bang_delay(&self) -> bool;        // Claude: delay after `!`
    fn needs_auto_status(&self) -> bool;       // workaround for broken hooks
    fn skip_permissions_flag(&self) -> Option<&'static str>;  // --dangerously-skip-permissions
    fn prompt_flag(&self) -> Option<&'static str>;            // e.g. -p / --prompt
    fn prompt_argument(&self, prompt_path: &str) -> String;
    fn default_subcommand(&self) -> Option<&'static str>;     // e.g. "chat" for kiro-cli
    fn auto_name_command(&self) -> Option<&'static str>;
    fn continue_flag(&self) -> Option<&'static str>;          // --continue / --resume
    fn model_flag(&self) -> Option<&'static str>;             // --model / -m
    fn permission_mode_flags(&self, mode: &str) -> Option<Vec<String>>;
}
```

Every method but `name` has a default, so adding an agent means overriding only
what differs; all pipeline/orchestrator/harness behaviors adapt automatically.

### Agent bootstrap (`src/bootstrap.rs`)

Uniform theme/plugin/skill/subagent/prompt installation across all supported
agents from `.muxix.yaml`:

```yaml
bootstrap:
  theme: catppuccin                     # per-agent map also allowed
  default_plugins:
    - npm:pi-web-access
  default_skills:
    - ./skills/worktree                 # local paths only (remote is skipped)
  default_subagents:
    - ./agents/reviewer.md
    - name: planner                     # or inline
      description: Plans work before execution
      model: haiku                      # resolved against `providers:`
      prompt: |
        You are a planning specialist...
  default_prompt_components:            # .muxix/prompt-components/<name>.md
    - fff
  template_vars:                        # rendered into each SKILL.md
    review_cmd:
      claude: /code-review
      default: /review
  features:                             # plugin where available, prompt otherwise
    ponytail:
      pi: git:github.com/DietrichGebert/ponytail
      default: ponytail
  agents:
    claude code:
      default_provider: anthropic
      additional_prompt_components: [code-review]
      subagent_models:
        explore: haiku
```

`muxix setup` reads this and applies it to each detected agent. Full reference:
[docs/guide/bootstrap.md](./guide/bootstrap.md).

---

## MCP Server Management

Declarative MCP (Model Context Protocol) server config in `.muxix.yaml`, rendered into each worktree's `.mcp.json`.

```yaml
# .muxix.yaml
mcp:
  socraticode:
    command: npx
    args: ["-y", "socraticode"]
  my-custom-server:
    command: node
    args: ["./tools/mcp-server.js"]
    enabled: false   # disable without removing
  taskflow:
    command: npx
    args: ["-y", "-p", "opencode-taskflow", "opencode-taskflow-mcp"]
    agents: [opencode]   # render only into this agent's config
```

```bash
# Render .mcp.json from config, propagate to worktrees
muxix mcp sync

# Show configured servers and integration status
muxix mcp status
```

The render is a **merge**: `x-muxix-managed` array tracks which keys muxix owns. Removing/disabling a server in config removes it from the file; hand-added entries are preserved.

`agents:` limits a server to the listed agents (short id like `pi` or lowercased display name like `claude code`); absent means all. Caveat: Claude, pi, and omp all read the shared project `.mcp.json`, so a server scoped to any one of them is still visible to the others through that file.

`.mcp.json` is propagated into each worktree as a relative symlink during `muxix add`.

---

## Microsandbox (libkrun microVM)

Alternative sandbox backend via the `msb` CLI (libkrun-based). Each agent runs in an isolated microVM with:
- Snapshot/restore in **<100ms** (vs CRIU seconds)
- **No root or TAP setup** — networking via TSI (Transparent Socket Impersonation)
- Compatible with `pipeline` context inheritance via `Transfer::Snapshot`

```yaml
# .muxix.yaml
sandbox:
  backend: microsandbox
  microsandbox:
    image: muxix-claude:latest
    memory_mb: 4096
  checkpoint:
    strategy: microsandbox
    dir: .muxix/checkpoints
```

```bash
# Install msb CLI
muxix sandbox setup microsandbox

# Run agent in microsandbox
muxix add my-feature --sandbox
```

Install detection: checks `PATH`, then `~/.local/bin/msb`, `~/.microsandbox/bin/msb`, `/usr/local/bin/msb`.

Dockerfile: `docker/Dockerfile.microsandbox`

---

## Sandbox Checkpoint

Checkpoint a running sandbox agent and restore it later (pipeline context inheritance via snapshot).

```yaml
sandbox:
  checkpoint:
    strategy: microsandbox  # or: container (CRIU), lima (unsupported)
    dir: .muxix/checkpoints
```

| Backend | Mechanism | Notes |
|---|---|---|
| `microsandbox` | `msb sandbox snapshot` | <100ms, preferred |
| `container` | `docker checkpoint create` (CRIU) | requires CRIU installation |
| `lima` | _(not supported)_ | returns Ok, no-op |

Checkpoint IDs and timestamps are recorded in agent state (`StateStore`). Pipeline runner reads checkpoint on `Inherit+Snapshot` nodes to restore context.

---

## Project State Store

Per-project key-value store for runtime state that survives across muxix restarts. Used internally by the setup phase and Project tab.

```bash
# Read a capability flag
muxix project-state get mcp.synced

# Set a capability
muxix project-state set test_env ready

# List all entries
muxix project-state list
```

Stored at `.muxix/capabilities.json`. Format: `{ "key": { "value": "...", "updated_at": 1234567890 } }`.

The `CapabilityStatus` enum: `Unknown` / `Pending` / `Ready` / `Failed`.

---

## Event Tracing

Structured JSON event log written to `~/.local/state/muxix.log`. Records every
agent pane interaction, signal, and turn for post-hoc debugging.

```bash
# Tail live events
tail -f ~/.local/state/muxix.log | jq 'select(.fields.ev != null)'

# Follow one pane
cat ~/.local/state/muxix.log | jq 'select(.fields.pane == "%12")'
```

### Event format

```json
{"timestamp":"2026-06-19T10:23:01Z","level":"INFO","target":"wm::event",
 "fields":{"ev":"turn.start","pane":"%12"},"spans":[{"node":"implement","kind":"agent"}]}
```

### Configuration

```yaml
# .muxix.yaml
events:
  enabled: true
  level: debug          # off | info | debug | trace
  disable:
    - pane.probe        # silence high-frequency poll group
    - turn.poll
  only: []              # allowlist (non-empty = emit only these)
```

Or via env:
```bash
MUXIX_LOG_FORMAT=text muxix open feature   # human-readable
MUXIX_EVENTS=debug muxix open feature      # high-frequency events
MUXIX_EVENTS=off muxix open feature        # silence events
```

---

## Additional CLI Commands (not in upstream)

| Command | Description |
|---|---|
| `muxix signal <kind>` | Emit pane or node signal |
| `muxix task <sub>` | CLI CRUD for task graph |
| `muxix notify` | Signal task completion to orchestrator |
| `muxix mcp sync` | Render `.mcp.json` from config |
| `muxix mcp status` | Show MCP server status |
| `muxix project-state` | Read/write project state store |
| `muxix focus <target>` | Switch to agent by ID or name fragment |
| `muxix clean` | Mark stuck tasks done (no active implementation) |
| `muxix hooks-report` | Emit hooks capability report to event log |
| `muxix provision [status\|sync]` | Sync org policy from a provision server |
| `muxix profile <show\|export\|diff>` | Inspect the sanitised config snapshot |

---

## Agent Cross-Inputs (in progress)

Mechanism for passing captured agent output from one pipeline node (or task) into subsequent nodes' prompts. Enables multi-stage pipelines where the implement stage consumes the plan stage's output without the agent having to re-read it from disk.

### Intra-harness output injection ✓ (implemented)

Within a single harness run, any node's captured stdout is available to later nodes via `{{outputs.<node_id>}}` placeholder substitution in `prompt` or `prompt_file`.

```yaml
nodes:
  - id: plan
    prompt: |
      Analyse the task and write a detailed implementation plan.
      Wrap it between ===MUXIX-PLAN-BEGIN=== and ===MUXIX-PLAN-END===.
    record_plan: true    # also persists to task graph's implementation_plan field

  - id: implement
    depends_on: [plan]
    prompt: |
      Here is the approved plan:

      {{outputs.plan}}

      Implement it now. Do not re-plan.
```

`{{outputs.<id>}}` is substituted at prompt-dispatch time. Missing node IDs log a warning and expand to empty string (prompt still dispatches).

**Output persistence** (`outputs_file`): captured outputs are atomically written to `.muxix/runs/<task-id>/outputs.json` after each node completes. On harness restart the file is loaded first, so `{{outputs.plan}}` still resolves even if the `plan` node ran in a previous process.

```json
// .muxix/runs/auth-module/outputs.json
{
  "plan": "1. Add JWT signing in auth/jwt.ts\n2. ...",
  "test": "All 14 tests passed."
}
```

**Feedback injection** (rejected breakpoints): when a human rejects a gate with feedback (`/reject "needs more detail on error handling"`), the feedback text is prepended to the retry node's resolved prompt automatically — no `{{outputs.*}}` placeholder needed.

### Cross-task output passing ✗ (in progress)

The `provides`/`read_set` fields on `GraphTask` are implemented for **conflict detection and dependency ordering** (see §6), but runtime routing of task A's output into task B's harness prompt is not yet wired up.

Planned mechanism:
- The orchestrator reads `outputs.json` from a completed task's run directory
- Before spawning a dependent task, it injects matching `provides` → `read_set` outputs into the new task's harness as template variables (`{{task.<id>.output.<key>}}`)
- The dependent task's `prompt_file` can reference those variables without filesystem coordination

Current workaround — explicit file reference in prompt:
```yaml
# task B's harness prompt
prompt: |
  Task A produced the following interface contract:
  {{bash: cat .muxix/runs/task-a/outputs.json | jq -r '.plan'}}

  Implement the consumer side.
```

### `record_plan` and `implementation_plan` field

Nodes with `record_plan: true` trigger special extraction: the runner looks for `===MUXIX-PLAN-BEGIN===` / `===MUXIX-PLAN-END===` delimiters in the output; falling back to the full output. The extracted text is:
1. Stored in `node_outputs["plan"]` (available as `{{outputs.plan}}`)
2. Written to `outputs.json`
3. Persisted onto the task graph entry as `implementation_plan`

This makes the plan visible in the Tasks tab, the dashboard, and `GET /tasks/:id` without any extra tooling.

```json
// tasks/index.json (after plan stage)
{
  "id": "auth-module",
  "status": "in_progress",
  "implementation_plan": "1. Add JWT signing in auth/jwt.ts\n..."
}
```

### `GET /agents` endpoint

---

## Proxy Chain (in progress)

Per-worktree agentgateway + RTK proxy chain. Each worktree gets a dedicated port (deterministic from handle hash, base + hash % 1000).

```yaml
# .muxix.yaml
proxy_chain:
  enabled: false    # not yet implemented; opt-in gated
  hops:
    - name: gateway
      type: gateway
    - name: rtk
      type: rtk
      endpoint: http://localhost:8888
    - name: claude-api
      type: claude-api
      endpoint: https://api.anthropic.com
```

Status: scaffolded (`src/proxy/`), gated off by default. Follow-up task: `finish-proxy-chain`.

---

## Org Provisioning & Policy

`muxix provision` syncs the machine with an org provision server: it pushes a
sanitised profile snapshot, fetches an org policy, caches it, audits the merged
config against it, and wires local agents to the org's governed gateway.

```bash
muxix provision              # = provision status (offline, reads the cache)
muxix provision sync         # push profile -> fetch policy -> cache -> audit
muxix provision sync --strict   # non-zero exit on any violation (CI gate)
muxix provision --dry-run    # audit only, write nothing
```

Config is **global-only** (`~/.config/muxix/config.yaml`); env vars win over it
so a container needs no config file:

```yaml
provision:
  server_url: https://policy.corp.example.com  # env: MUXIX_PROVISION_URL
  token_path: ~/.config/muxix/provision-token  # env: MUXIX_PROVISION_TOKEN; file must be mode 600
  grace_period_secs: 259200                 # use an expired policy for 72h more
```

`sync_on_setup`, `policy_ttl_secs`, and `insecure_skip_tls` parse but are inert
today — `muxix setup` never syncs, and the TTL comes from the server response.

### Policy enforcement is at config load, not at sync

`Config::load` reads `~/.config/muxix/policy.yaml` on **every** command, merges
the locked fields over user config, and reports violations. A policy therefore
governs `add`, `dashboard`, `pipeline`, etc. without re-syncing.

| Policy field | Effect |
|---|---|
| `locked.proxy_chain` | Replaces `proxy_chain`; a local value is reported as overridden |
| `locked.sandbox_network` | Replaces `sandbox.network` |
| `forbidden_mcp_commands` | Substring patterns banned from any MCP `command` |
| `allowed_mcp_servers` | Allowlist of MCP server names |
| `allowed_agent_kinds` | Restricts `agent:` to those CLI stems |
| `deny_external_providers` + `allowed_providers` | Allowlist for the `providers:` registry |
| `violation_severity` | `warn` (default) or `error` |

Locked fields apply regardless of severity; severity only decides how loudly.
Cache states: `Fresh` (within TTL) → `Stale` (within grace, warns) → `Expired`
(dropped) → `Missing` (no-op).

### Gateway wiring

When the fetch response carries a `gateway`, `sync` writes native provider config
so agent traffic routes through it — no hand-set `ANTHROPIC_BASE_URL`:

| Agent | File | Written |
|---|---|---|
| OpenCode | `~/.config/opencode/opencode.json` | `provision-gateway` provider (`@ai-sdk/openai-compatible`), `baseURL: <gw>/v1`, `model` when unset |
| Claude Code | `~/.claude/settings.json` | `env.ANTHROPIC_BASE_URL` + `apiKeyHelper` |

Secrets never land on disk: the token is referenced by env var name
(`{env:VAR}` for OpenCode, `apiKeyHelper` for Claude), defaulting to
`MUXIX_PROVISION_TOKEN`. Both files are merged, not rewritten.

### Profile snapshot (`muxix profile`)

Secret-free by construction — names and flags only, no commands, keys, or env
values.

```bash
muxix profile show                 # YAML to stdout
muxix profile export -o me.yaml
muxix profile diff                 # reports the policy's team_profile_url (diff TBD)
```

Fields: `muxix_version`, `platform`, `agent_kind` (CLI stem only), `mcp_names`,
`provider_names`, `features` (`sandbox_enabled` / `proxy_chain_enabled` /
`bootstrap_enabled`), and a djb2 `hostname_hash`.

Artifacts: `~/.config/muxix/policy.yaml` (cache),
`~/.local/state/muxix/provision-audit.jsonl` (one JSON line per sync).

Full reference: [docs/reference/commands/provision.md](./reference/commands/provision.md).

---

