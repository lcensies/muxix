---
description: Detect installed coding agents and install status hooks, skills, plugins, prompt components, and MCP servers
---

# setup

Detects the coding agents you have installed and applies muxix's per-agent
configuration to each of them: status-tracking hooks, skills, and the
[`bootstrap`](../../guide/bootstrap.md) manifest (plugins, prompt components,
features), plus [MCP](../../guide/configuration.md) server sync.

```bash
muxix setup
```

With no flags `setup` is **interactive** — it requires a terminal, builds the
same drift report `--check` would print, and asks for one confirmation before
applying every selected section. It is safe to re-run; installation is
idempotent and re-running merges in anything newly added (for example new
hooks after a muxix update). If nothing has drifted, it says so and exits
without prompting. Use `--only` (or the `--hooks`/`--skills` aliases) to scope
an interactive run to specific sections instead of confirming all of them.

## Options

| Flag                       | Description                                                        |
| -------------------------- | ------------------------------------------------------------------ |
| `--non-interactive`, `-y`  | Apply every configured section without prompting. Does not need a terminal. |
| `--check`                  | Report what would change, write nothing, exit 2 if the machine has drifted |
| `--json`                   | Emit one JSON object on stdout; progress goes to stderr             |
| `--only <sections>`        | Restrict to these sections, comma-separated                         |
| `--profile <names>`        | Resolve config with these [profiles](../../guide/profiles.md) selected |
| `--no-prune`               | Keep harness features muxix installed that the config no longer declares |
| `--hooks`                  | Alias for `--only hooks`                                            |
| `--skills`                 | Alias for `--only skills`                                           |

Sections: `hooks`, `skills`, `subagents`, `plugins`, `agent-settings`, `providers`, `prompts`, `theme`, `mcp`, `agent-profiles`, `deps`.
With no `--only`, all of them run, in that order.

## Unattended use

`--non-interactive` is what makes `setup` usable from an activation script, a
devcontainer `postCreate`, or CI:

```bash
muxix setup --non-interactive
muxix setup --non-interactive --only hooks,mcp --profile corp
```

A failing item never aborts the run — every remaining section is still applied,
and the failure surfaces in the exit code.

## Checking for drift

`--check` runs the same code path with every write suppressed, so what it
reports is exactly what a real run would do:

```bash
muxix setup --check                # exit 0 in sync, 2 if drifted
muxix setup --check --only skills
```

Two sections cannot be inspected without acting, and say so rather than
claiming "no drift": **prompts**, **theme**, and **mcp** (no read-back). They report
`skipped` with a reason, and interactive setup still offers to apply them when
nothing else drifted. Plugins are probed against each agent's own installed
state, so they report real drift.

## Outcomes and exit codes

Every item reports one of `installed`, `updated`, `up-to-date`, `removed`,
`skipped`, or `failed`. An agent whose CLI is not installed reports `skipped`
with a reason — not a failure.

| Exit | Meaning |
| ---- | ------- |
| `0`  | Applied cleanly, or `--check` found no drift |
| `1`  | At least one item failed |
| `2`  | `--check` only: the machine has drifted from the declared harness |

`--json` carries the same information structurally, for callers that would
rather not read exit codes:

```bash
muxix setup --check --json | jq '.items[] | select(.outcome != "up-to-date")'
```

`--hooks` and `--skills` restrict which sections run, same as `--only hooks`
and `--only skills`. MCP server sync is its own `mcp` section, so a scoped
`--hooks` or `--skills` run does not touch it.

## Removing what the config dropped

Deleting a skill, subagent, plugin, or hook from `.muxix.yaml` and re-running
`setup` takes it off the machine; the item reports `removed`, and `--check`
counts it as drift. Only features muxix itself installed (tracked in
`$XDG_STATE_HOME/muxix/managed.json`) are ever removed, pruning is scoped to
the project that declared them, and `--only` limits it to the sections that
ran. `--no-prune` converges without removing anything. See
[bootstrap → Removing a feature](../../guide/bootstrap.md#removing-a-feature).

## What it does

`setup` runs these phases in order. Each phase only touches the agents it detects
on your machine.

### 1. Agent detection

Detects each agent by looking for its configuration directory or CLI:

| Agent       | Detected via                                                    |
| ----------- | -------------------------------------------------------------- |
| Claude Code | `~/.claude/` (or `CLAUDE_CONFIG_DIR`)                          |
| Codex       | `~/.codex/`                                                    |
| Copilot CLI | Copilot config directory                                       |
| Gemini CLI  | Gemini config directory                                        |
| OpenCode    | OpenCode config directory                                      |
| pi          | `~/.pi/agent/` (or `PI_CODING_AGENT_DIR`)                      |
| omp         | `~/.omp/agent/` (`PI_CODING_AGENT_DIR`, or `OMP_CODING_AGENT_DIR` to point muxix alone) |

If no agents are detected, `setup` exits with a hint to install an agent CLI.

### 2. Status-tracking hooks (`--hooks`)

Installs the hooks that report agent status into your tmux window list. See
[Status tracking](../../guide/status-tracking.md) for what each agent supports
and any agent-specific requirements (e.g. Codex needs `hooks = true` in
`~/.codex/config.toml`). Muxix also adjusts your tmux `window-status-format`
once per session to render the status icons.

### 3. Skills (`--skills`)

Installs the **bundled** muxix skills into every detected agent that has a
skills directory. Skill installation failures do not abort the rest of setup.
Project-declared skills (`bootstrap.skills`) are installed in the
bootstrap phase below, not here.

### 4. Bootstrap (theme, plugins, skills, subagents, prompt components)

If a [`bootstrap`](../../guide/bootstrap.md) section is present and non-empty,
its sections are included in the same drift report and single confirmation
described above, applied in this order:

- **Theme** is written into each agent's own config (Claude Code, Gemini CLI,
  OpenCode); see [Theme](../../guide/bootstrap.md#theme).
- **Plugins** are installed via each agent's own installer (`pi install`,
  `omp install`, `claude plugin install`, `opencode plugin`). Spec formats
  differ per agent; see [Plugins](../../guide/bootstrap.md#plugins). Codex,
  Copilot, and Gemini have no plugin installer wired up and are skipped — a
  feature falls back to their prompt component instead.
  When hooks are declared for OpenCode or pi, their Claude-hooks-compat
  plugin is auto-added to this list (reported `auto-added`); see
  [Skill hooks](../../guide/bootstrap.md#skill-hooks). Copilot CLI has its own
  hook file and needs no plugin; omp has neither and is reported `skipped`.
- **Skills** from `skills` / `add_skills` are copied into each
  agent's skills directory, with `SKILL.md` rendered per host agent against
  [`template_vars`](../../guide/bootstrap.md#skill-template-variables).
- **Subagents** from `subagents` / `add_subagents` are installed
  into each agent's native subagents directory, with `model:` resolved against
  the [`providers:`](../../guide/models.md) registry. Copilot uses its own
  `<name>.agent.md` naming; Codex is reported `skipped` because its custom
  agents are TOML config layers, not markdown documents.
- **Prompt components** from `.muxix/prompt-components/` are merged into each
  agent's system prompt.
- **Features** resolve per agent to either a plugin or a `default` prompt
  component; see [Features](../../guide/bootstrap.md#features).

The bootstrap phase runs when any of `plugins`, `skills`,
`subagents`, `prompt_components`, `features`, `theme`, or
per-agent `agents` overrides are set.

Individual failures are reported per item and do not abort the phase — setup
continues with the next plugin, skill, subagent, or agent.

### 5. MCP server sync

When `mcp` servers are configured, the `mcp` section renders them
into each detected agent's native config and pre-approves them in that agent's
trust mechanism, so a harness-launched agent never blocks on an interactive
"trust this MCP server?" prompt at startup. Claude, pi, omp and Copilot share
the project `.mcp.json`; Gemini uses `.gemini/settings.json`, OpenCode
`opencode.json`, and Codex a managed `[mcp_servers.*]` region in
`.codex/config.toml` plus the project-trust entry it needs to read that layer.

### 6. Dependencies (`deps`)

For every skill or MCP entry with a `requires:` block: install missing or
mismatched npm packages into `bootstrap.npm_prefix`, assert `bin` names on
PATH. Runs last. Outcomes: `installed`/`updated` (npm), `up-to-date`, `failed`
(missing binary, npm absent, conflicting pins), `removed` (pruned package).
See [Dependencies](../../guide/bootstrap.md#dependencies).

## Examples

```bash
# Full setup: hooks + skills + bootstrap + MCP sync
muxix setup

# Only (re)install status-tracking hooks
muxix setup --hooks

# Only (re)install skills
muxix setup --skills
```

## See also

- [Agent bootstrap](../../guide/bootstrap.md) — the manifest `setup` applies
- [provision](./provision.md) — org policy sync (separate command; `setup` never syncs)
- [Models](../../guide/models.md) — unified provider/model registry
- [Status tracking](../../guide/status-tracking.md) — hooks and agent support
- [init](./init.md) — generate a starter `.muxix.yaml`
