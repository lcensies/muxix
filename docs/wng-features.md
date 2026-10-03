# wng — Extended Features over Workmux

This document covers features **added in wng that do not exist in upstream workmux**.
Upstream provides: worktree lifecycle (add/open/close/merge/remove), sidebar, dashboard (Agents + Worktrees tabs), sandbox (Lima/container), status tracking, multiplexer support (tmux/wezterm/kitty/zellij).

---

## Extended Dashboard TUI — Tasks Tab

Upstream has two dashboard tabs: **Agents** and **Worktrees**. wng adds **Tasks**,
a view over the project's task graph (`tasks/index.json`).

```bash
workmux dashboard              # opens on Agents tab
workmux dashboard -t tasks     # open directly on Tasks
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

### CLI (`workmux task`)

```bash
# List all tasks
workmux task list

# Filter by status
workmux task list --status todo

# Show only frontier (ready to start)
workmux task list --frontier

# JSON output
workmux task list --json

# Get a single task
workmux task get auth-module

# Create a task
workmux task create \
  --id auth-module \
  --title "Implement auth" \
  --description "JWT-based auth with refresh tokens" \
  --depends-on ""  # comma-separated IDs

# Update fields
workmux task update auth-module --status in_progress
workmux task update auth-module --title "Auth module (revised)"

# Delete
workmux task delete auth-module
```

Relative `--graph` paths resolve to the **main worktree root**, so agents running inside a feature worktree automatically share the project's single graph.

## Signal System

Out-of-band agent signaling — an external harness reads these instead of parsing agent output.

```bash
workmux signal <kind> [--pane <pane-id>] [--node <node-id>] [--feedback "..."]
```

### Pane-keyed signals (agent lifecycle hooks)

Keyed by `$TMUX_PANE` (or `--pane`). Written to `.workmux/signals/<pane-id>/<kind>`.

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
    "Stop": [{"matcher": "", "hooks": [{"type": "command", "command": "workmux signal turn-done"}]}],
    "Notification": [{"matcher": "", "hooks": [{"type": "command", "command": "workmux signal needs-input"}]}]
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
workmux signal done --node implement

# On failure:
workmux signal error --node implement --feedback "Tests failed: 3 assertions"
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
agents from `.workmux.yaml`:

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
  default_prompt_components:            # .workmux/prompt-components/<name>.md
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

`workmux setup` reads this and applies it to each detected agent. Full reference:
[docs/guide/bootstrap.md](./guide/bootstrap.md).

---

## MCP Server Management

Declarative MCP (Model Context Protocol) server config in `.workmux.yaml`, rendered into each worktree's `.mcp.json`.

```yaml
# .workmux.yaml
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
workmux mcp sync

# Show configured servers and integration status
workmux mcp status
```

The render is a **merge**: `x-workmux-managed` array tracks which keys workmux owns. Removing/disabling a server in config removes it from the file; hand-added entries are preserved.

`agents:` limits a server to the listed agents (short id like `pi` or lowercased display name like `claude code`); absent means all. Caveat: Claude, pi, and omp all read the shared project `.mcp.json`, so a server scoped to any one of them is still visible to the others through that file.

`.mcp.json` is propagated into each worktree as a relative symlink during `workmux add`.

---

## Microsandbox (libkrun microVM)

Alternative sandbox backend via the `msb` CLI (libkrun-based). Each agent runs in an isolated microVM with:
- Snapshot/restore in **<100ms** (vs CRIU seconds)
- **No root or TAP setup** — networking via TSI (Transparent Socket Impersonation)
- Compatible with `pipeline` context inheritance via `Transfer::Snapshot`

```yaml
# .workmux.yaml
sandbox:
  backend: microsandbox
  microsandbox:
    image: workmux-claude:latest
    memory_mb: 4096
  checkpoint:
    strategy: microsandbox
    dir: .workmux/checkpoints
```

```bash
# Install msb CLI
workmux sandbox setup microsandbox

# Run agent in microsandbox
workmux add my-feature --sandbox
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
    dir: .workmux/checkpoints
```

| Backend | Mechanism | Notes |
|---|---|---|
| `microsandbox` | `msb sandbox snapshot` | <100ms, preferred |
| `container` | `docker checkpoint create` (CRIU) | requires CRIU installation |
| `lima` | _(not supported)_ | returns Ok, no-op |

Checkpoint IDs and timestamps are recorded in agent state (`StateStore`). Pipeline runner reads checkpoint on `Inherit+Snapshot` nodes to restore context.

---

## Project State Store

Per-project key-value store for runtime state that survives across workmux restarts. Used internally by the setup phase and Project tab.

```bash
# Read a capability flag
workmux project-state get mcp.synced

# Set a capability
workmux project-state set test_env ready

# List all entries
workmux project-state list
```

Stored at `.workmux/capabilities.json`. Format: `{ "key": { "value": "...", "updated_at": 1234567890 } }`.

The `CapabilityStatus` enum: `Unknown` / `Pending` / `Ready` / `Failed`.

---

## Event Tracing

Structured JSON event log written to `~/.local/state/workmux.log`. Records every
agent pane interaction, signal, and turn for post-hoc debugging.

```bash
# Tail live events
tail -f ~/.local/state/workmux.log | jq 'select(.fields.ev != null)'

# Follow one pane
cat ~/.local/state/workmux.log | jq 'select(.fields.pane == "%12")'
```

### Event format

```json
{"timestamp":"2026-06-19T10:23:01Z","level":"INFO","target":"wm::event",
 "fields":{"ev":"turn.start","pane":"%12"},"spans":[{"node":"implement","kind":"agent"}]}
```

### Configuration

```yaml
# .workmux.yaml
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
WORKMUX_LOG_FORMAT=text workmux open feature   # human-readable
WORKMUX_EVENTS=debug workmux open feature      # high-frequency events
WORKMUX_EVENTS=off workmux open feature        # silence events
```

---

## Additional CLI Commands (not in upstream)

| Command | Description |
|---|---|
| `workmux signal <kind>` | Emit pane or node signal |
| `workmux task <sub>` | CLI CRUD for task graph |
| `workmux notify` | Signal task completion to orchestrator |
| `workmux mcp sync` | Render `.mcp.json` from config |
| `workmux mcp status` | Show MCP server status |
| `workmux project-state` | Read/write project state store |
| `workmux focus <target>` | Switch to agent by ID or name fragment |
| `workmux clean` | Mark stuck tasks done (no active implementation) |
| `workmux hooks-report` | Emit hooks capability report to event log |
| `workmux provision [status\|sync]` | Sync org policy from a provision server |
| `workmux profile <show\|export\|diff>` | Inspect the sanitised config snapshot |

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
      Wrap it between ===WORKMUX-PLAN-BEGIN=== and ===WORKMUX-PLAN-END===.
    record_plan: true    # also persists to task graph's implementation_plan field

  - id: implement
    depends_on: [plan]
    prompt: |
      Here is the approved plan:

      {{outputs.plan}}

      Implement it now. Do not re-plan.
```

`{{outputs.<id>}}` is substituted at prompt-dispatch time. Missing node IDs log a warning and expand to empty string (prompt still dispatches).

**Output persistence** (`outputs_file`): captured outputs are atomically written to `.workmux/runs/<task-id>/outputs.json` after each node completes. On harness restart the file is loaded first, so `{{outputs.plan}}` still resolves even if the `plan` node ran in a previous process.

```json
// .workmux/runs/auth-module/outputs.json
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
  {{bash: cat .workmux/runs/task-a/outputs.json | jq -r '.plan'}}

  Implement the consumer side.
```

### `record_plan` and `implementation_plan` field

Nodes with `record_plan: true` trigger special extraction: the runner looks for `===WORKMUX-PLAN-BEGIN===` / `===WORKMUX-PLAN-END===` delimiters in the output; falling back to the full output. The extracted text is:
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
# .workmux.yaml
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

`workmux provision` syncs the machine with an org provision server: it pushes a
sanitised profile snapshot, fetches an org policy, caches it, audits the merged
config against it, and wires local agents to the org's governed gateway.

```bash
workmux provision              # = provision status (offline, reads the cache)
workmux provision sync         # push profile -> fetch policy -> cache -> audit
workmux provision sync --strict   # non-zero exit on any violation (CI gate)
workmux provision --dry-run    # audit only, write nothing
```

Config is **global-only** (`~/.config/workmux/config.yaml`); env vars win over it
so a container needs no config file:

```yaml
provision:
  server_url: https://sc.corp.example.com   # env: WORKMUX_SC_URL
  token_path: ~/.config/workmux/sc-token    # env: WORKMUX_SC_TOKEN; file must be mode 600
  grace_period_secs: 259200                 # use an expired policy for 72h more
```

`sync_on_setup`, `policy_ttl_secs`, and `insecure_skip_tls` parse but are inert
today — `workmux setup` never syncs, and the TTL comes from the server response.

### Policy enforcement is at config load, not at sync

`Config::load` reads `~/.config/workmux/policy.yaml` on **every** command, merges
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
`WORKMUX_SC_TOKEN`. Both files are merged, not rewritten.

### Profile snapshot (`workmux profile`)

Secret-free by construction — names and flags only, no commands, keys, or env
values.

```bash
workmux profile show                 # YAML to stdout
workmux profile export -o me.yaml
workmux profile diff                 # reports the policy's team_profile_url (diff TBD)
```

Fields: `workmux_version`, `platform`, `agent_kind` (CLI stem only), `mcp_names`,
`provider_names`, `features` (`sandbox_enabled` / `proxy_chain_enabled` /
`bootstrap_enabled`), and a djb2 `hostname_hash`.

Artifacts: `~/.config/workmux/policy.yaml` (cache),
`~/.local/state/workmux/provision-audit.jsonl` (one JSON line per sync).

Full reference: [docs/reference/commands/provision.md](./reference/commands/provision.md).

---

