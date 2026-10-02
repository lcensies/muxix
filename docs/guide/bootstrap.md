---
description: Configure plugins, skills, prompt components, and cross-agent features once and apply them to every coding agent
---

# Agent bootstrap

The `bootstrap` section of your project config (`.workmux.yaml`) is a single
manifest that configures **plugins**, **skills**, **subagents**, **prompt
components**, **features**, and the **theme** uniformly across every coding agent
(Claude Code, Codex, Copilot, Gemini, OpenCode, pi, omp). Running
[`workmux setup`](../reference/commands/setup.md) detects the agents you have
installed and applies the manifest to each of them.

```yaml
bootstrap:
  default_prompt_components:
    - fff
  features:
    ponytail:
      pi: git:github.com/DietrichGebert/ponytail
      default: ponytail
  agents:
    claude code:
      additional_prompt_components:
        - code-review
    pi:
      additional_plugins:
        - npm:pi-web-access
```

Everything in `bootstrap` is opt-in and backwards compatible: projects without a
`bootstrap` section are unaffected, and each field defaults to empty.

::: tip Per-machine and per-org variation
A bootstrap block does not have to be one-size-fits-all. Use
[profiles and includes](/guide/profiles) to keep a corporate machine, a personal
one, and a per-language stack in the same config, selected with `--profile`.
:::

## Skill hooks

A skill that ships executable scripts can bind them to agent lifecycle events,
so the harness installs the hook deterministically instead of asking the agent
to do it:

```yaml
# ~/.config/workmux/config.yaml -- hooks are global-only (see below)
bootstrap:
  default_skills:
    - path: ./skills/auto-git
      hooks:
        turn-done:
          - command: bash "{{ skill_install_dir }}/scripts/autocommit.sh"
            sha256: 67cf646abe2c3854d85a2c35bf5aa218eb4c07779c0bb7efe8cbe708cd0ad757
```

Events are agent-agnostic — `session-ready` and `turn-done` — and translate to
each agent's native mechanism: Claude Code `SessionStart`/`Stop`, Codex `Stop`
(it has no per-session event), Gemini `BeforeAgent`/`AfterAgent`. OpenCode and
pi have no shell-hook config of their own, so workmux writes Claude-format
hooks for their compat plugins — [`opencode-claude-hooks`](https://github.com/magarcia/opencode-claude-hooks)
(`~/.config/opencode/hooks.json`) and [`@hsingjui/pi-hooks`](https://github.com/hsingjui/pi-hooks)
(`~/.pi/agent/settings.json`) — and auto-adds the plugin to that agent's
install list when hooks are declared (reported as `auto-added`, suppressed by
any existing entry matching the plugin name). Two caveats: the OpenCode plugin
also reads `.claude/settings*.json` and concatenates all files, so a hook
installed for Claude Code may fire in OpenCode sessions too — write hook
scripts to tolerate running twice per event (the auto-git sweeper does); and
omp/Copilot still report `skipped` in `workmux setup`, so that gap stays
visible rather than silent.

Hooks not tied to any skill go under `bootstrap.hooks` with the same shape;
`{{ skill_install_dir }}` has no meaning there and is a render error.

**Hooks are global-only.** A project config declaring one gets it stripped with
a warning, the same as `agents:`. A shipped SKILL.md is text an agent may act
on; a hook is a command the harness runs unattended on every turn — cloning a
repository and running `workmux setup` must not execute that repository's
script on your machine. The repo may ship the skill; turning the hook on is
yours.

`sha256` is optional and pins the script: `workmux setup` hashes the installed
copy before writing the hook and fails closed on mismatch, printing both
digests (so the workflow is: omit it, run once, paste what it reports). The pin
covers install time; ongoing tamper detection is the skills section's content
comparison, which reports a modified installed copy as drift in
`workmux setup --check`.

Removing a hook from config stops it being asserted but does not uninstall the
already-written entry — delete it from the agent's own config if you need it
gone.


## Dependencies

A skill's scripts or an MCP server's `command` often need something outside
the skill itself: a CLI from npm, `python3`, a binary the OS provides. Declare
it next to the entry, and `workmux setup` (section `deps`) makes it true or
says why it cannot:

```yaml
bootstrap:
  npm_prefix: ~/.local        # default; bins land in ~/.local/bin
  default_skills:
    - path: ./skills/openspec-taskflow
      requires:
        npm: ["@fission-ai/openspec@1.6.0"]   # installed by workmux, pinned
        bin: [python3]                         # asserted on PATH, never installed
mcp:
  context7:
    command: context7-mcp                      # the package's bin, not `npx -y`
    requires: { npm: ["@upstash/context7-mcp@2.1.0"] }
  fff-mcp:
    command: fff-mcp
    requires: { bin: [fff-mcp] }
```

- `npm`: `name@version` is an exact pin; a bare `name` installs the latest and
  is flagged `unpinned` on every run, never auto-updated. Installs go through
  `npm install -g --prefix <npm_prefix>`, so nothing touches a system or Nix
  store prefix. The same package declared by several entities is installed
  once; two different pins for one package fail with both entities named.
- `bin`: workmux only checks PATH. Missing → `[failed]`, exit non-zero, with
  the hint to provide it via the system (on Nix, `home.packages`).
- `--check` reports a missing/mismatched package or a missing binary as drift
  (exit 2) without running npm.
- Prune: when no entity declares a package workmux installed, the next setup
  uninstalls it (`--no-prune` keeps it). Packages you installed by hand under
  the prefix are left alone unless `bootstrap.deps_strict: true`, which makes
  the whole prefix declarative.

`requires` is not global-only: a project config may declare it, like it
already declares plugins and MCP commands.

## How resolution works

For a given agent, `workmux setup` computes four merged lists:

| List                  | Sources (in order)                                                                 |
| --------------------- | ---------------------------------------------------------------------------------- |
| **Plugins**           | `default_plugins` → `agents.<id>.additional_plugins` → `features` with an agent plugin |
| **Skills**            | `default_skills` → `agents.<id>.additional_skills`                                  |
| **Subagents**         | `default_subagents` → `agents.<id>.additional_subagents`                            |
| **Prompt components** | `default_prompt_components` → `features` falling back to `default` → `agents.<id>.additional_prompt_components`, minus `agents.<id>.disabled_prompt_components` |

The lists are de-duplicated and sorted before installation. [`theme`](#theme) is
resolved separately: it is a single value per agent, not a list.

### Agent keys

Anywhere an agent is referenced as a config key (`agents:` and per-agent entries
inside a feature), you can use either form:

| Agent       | Short id   | Display key      |
| ----------- | ---------- | ---------------- |
| Claude Code | `claude`   | `claude code`    |
| Codex       | `codex`    | `codex`          |
| Copilot CLI | `copilot`  | `copilot cli`    |
| Gemini CLI  | `gemini`   | `gemini cli`     |
| OpenCode    | `opencode` | `opencode`       |
| pi          | `pi`       | `pi`             |
| omp         | `omp`      | `omp`            |

Matching is case- and whitespace-insensitive, and applies to every per-agent
key: `agents:` entries, `features:` entries, and `default_provider`.

## Features

A **feature** decouples *what* you want ("enable ponytail") from *how* each agent
provides it. Different agents implement the same capability differently: one may
have a dedicated plugin, another may only take a system-prompt component. A
feature captures both in one place.

```yaml
bootstrap:
  features:
    ponytail:
      pi: git:github.com/DietrichGebert/ponytail   # pi installs a plugin
      omp: pi-ponytail@marketplace                 # omp installs its own spec
      default: ponytail                            # everyone else gets the prompt
    caveman:
      default: caveman-full                        # prompt-only feature
```

Resolution per agent:

1. If the feature declares a plugin for that agent (`pi:`, `omp:`, `claude code:`,
   …), the agent **installs that plugin** and does **not** get the prompt fallback.
2. Otherwise, if the feature has a `default`, the agent **merges that prompt
   component** (from `.workmux/prompt-components/<name>.md`).
3. Otherwise the feature is a no-op for that agent.

A feature's prompt fallback can still be suppressed for a specific agent via that
agent's `disabled_prompt_components`.

::: tip Why features exist
Before features, sharing a plugin list across agents (e.g. a YAML anchor from
`pi` to `omp`) broke because each agent's installer expects a different package
spec format. Features let you express the capability once and give each agent the
spec *it* understands — or fall back to a prompt where no plugin exists.
:::

## Plugins

Plugins are installed by shelling out to each agent's own installer, so the spec
string must match **that agent's** expected format. This is the most common
source of failed installs.

| Agent | Installer            | Accepted spec formats                                                     |
| ----- | -------------------- | ------------------------------------------------------------------------- |
| pi    | `pi install <spec>`  | `npm:@scope/pkg`, `git:github.com/user/repo`, `https://…`, `ssh://…`, `./path` |
| omp   | `omp install <spec>` | bare npm spec (`pkg@1.2.3`, `@scope/pkg`), marketplace ref (`name@marketplace`), `./path` |
| Claude Code | `claude plugin install <plugin> --scope user` | `<plugin>@<marketplace>`, optionally prefixed with a marketplace source and `#` (see below) |
| OpenCode | `opencode plugin <module> --global` | npm module name (`@scope/pkg`) |

### Claude Code marketplace sources

Claude installs in two steps — a marketplace must be registered before a plugin
from it can be installed — so a Claude spec may carry both, separated by `#`:

```yaml
claude code: DietrichGebert/ponytail#ponytail@ponytail
#            └─ marketplace source ─┘ └─ plugin@marketplace ─┘
```

workmux runs `claude plugin marketplace add DietrichGebert/ponytail` first, then
`claude plugin install ponytail@ponytail`. Drop the `#` prefix for a marketplace
that is already registered. Re-adding a registered marketplace is tolerated, so
`workmux setup` stays re-runnable.

::: tip Why the CLI, not settings.json
Declaring `enabledPlugins` / `extraKnownMarketplaces` in `settings.json` records
*intent* only: Claude Code
[does not load such a plugin until it is installed](https://code.claude.com/docs/en/discover-plugins#configure-team-marketplaces),
and instead prompts the user at the trust gate. That prompt would stall an
unattended harness agent before its first turn, so workmux shells out to the
installer.
:::

::: warning pi and omp specs are not interchangeable
`pi` uses **scheme-prefixed** specs (`npm:`, `git:`); `omp` uses **bare** npm
specs / marketplace refs with **no** prefix. Feeding pi-style `npm:foo` to omp
fails with `Invalid package name`. Give each agent its own `additional_plugins`
(or use a feature with per-agent entries) rather than sharing one list.
:::

Codex, Copilot, and Gemini have no plugin installer wired up today; a feature
falls back to their prompt component instead.

### Detecting already-installed plugins

Before shelling out to an installer, `workmux setup` probes whether a declared
plugin is already installed, so a converged machine reports `up-to-date`
instead of re-running the agent CLI:

- **pi and omp** — path-shaped specs (no `npm:`/`git:`/`https:`/`ssh:` scheme,
  e.g. `./vendor/x`) are canonicalized and compared as filesystem paths: the
  declared spec resolves against the project root, each recorded entry in the
  agent's `settings.json` `packages[]` resolves against the agent's own config
  dir (pi rewrites path specs relative to `~/.pi/agent`, so a naive string
  comparison never matches). Scheme-prefixed specs are compared as exact
  strings, unchanged from before.
- **Claude Code** — probed from `~/.claude/plugins/installed_plugins.json`
  (`{"plugins":{"<plugin>@<marketplace>":[{"scope":"user"}]}}`), matching the
  `plugin@marketplace` id (the part after `#`, if a marketplace source is
  present) against a `user`-scope entry — the scope `claude plugin install`
  always installs at. No `claude plugin list` call is made.

Both probes **fail open**: a missing or unreadable state file, unparsable
JSON, or a path that doesn't canonicalize (nothing installed there yet) all
read as not-installed, so setup still runs the install rather than silently
skipping a plugin that never landed.

## Prompt components

Prompt components are Markdown files. A bare name (without the `.md` extension)
loads `.workmux/prompt-components/<name>.md`; an entry containing `/` is a path to
a `.md` file (absolute, `~`, or relative to the project root), which lets a
machine-global config reference components kept outside any project. They are
merged into the agent's system prompt during setup. How the prompt is injected depends on the agent — for pi/omp
see [Injection method](#pi-and-omp-injection-method).

```yaml
bootstrap:
  default_prompt_components:
    - caveman-full     # .workmux/prompt-components/caveman-full.md
    - fff
    - ~/dotfiles/prompt-components/jj.md   # absolute paths work too
  agents:
    claude code:
      additional_prompt_components:
        - code-review
    pi:
      disabled_prompt_components:
        - caveman-full   # pi opts out of this one
```

## Theme

`theme` writes a color theme into each agent's own config during setup, so one
line keeps every agent looking the same:

```yaml
bootstrap:
  theme: catppuccin        # same theme for every agent
```

Theme names are **not** portable between agents, so the map form sets them per
agent, with an optional `default` for the rest:

```yaml
bootstrap:
  theme:
    claude code: dark      # Claude Code only ships its own dark/light variants
    default: catppuccin    # OpenCode: catppuccin, -mocha, -macchiato, -frappe, -latte
```

| Agent       | Written to                              | Key        |
| ----------- | --------------------------------------- | ---------- |
| Claude Code | `~/.claude/settings.json`               | `theme`    |
| Gemini CLI  | `~/.gemini/settings.json`               | `ui.theme` |
| OpenCode    | `~/.config/opencode/opencode.json`      | `theme`    |

Every other key in those files is preserved, and re-running setup is a no-op
when the theme already matches. Agents with no known theme setting (Codex,
Copilot, pi, omp) are skipped, as is any agent with no entry and no `default`.
The value is passed through as written — an unknown theme name is the agent's
own error to report.

::: warning OpenCode
OpenCode merges `config.json`, `opencode.json` and `opencode.jsonc` in that
order, so a `theme` in `opencode.jsonc` overrides what setup writes. Setup
reports this instead of writing a value that would never take effect.
:::

## Skills

Skills are directories containing a `SKILL.md`. Sources are local paths:

```yaml
bootstrap:
  default_skills:
    - ./shared/skills/my-skill                        # directory
    - ./shared/skills/other/SKILL.md                  # or the file itself
  agents:
    claude code:
      additional_skills:
        - ./skills/claude-only
```

The whole directory is copied into each agent's skills directory. Re-running
setup is a no-op when the source is unchanged.

::: warning Remote skill sources
The remote form (`url:` / `ref:`) parses but is **not fetched** — setup reports
it as skipped. Vendor the skill and point at a local path.
:::

### Skill template variables

`SKILL.md` is rendered per host agent as a [minijinja](https://docs.rs/minijinja)
template, so one skill source can say the right thing for each agent instead of
being forked per agent. Two variables are always present:

| Variable     | Value                                       |
| ------------ | ------------------------------------------- |
| `agent`      | Canonical short id — `claude`, `omp`, `pi`  |
| `agent_name` | Display name — `Claude Code`, `omp`         |

`template_vars` adds your own. A scalar applies to every agent; a map is keyed by
agent (the same [agent keys](#agent-keys) used elsewhere) with an optional
`default`:

```yaml
bootstrap:
  template_vars:
    orchestrator: orca            # same value for every agent
    review_cmd:
      claude: /code-review        # per agent
      default: /review
```

```markdown
<!-- SKILL.md -->

Run `{{ review_cmd }}` before handing work back, then report to {{ agent_name }}
via {{ orchestrator }}.
```

Rules that keep this from misfiring:

- A body with **no** `{{ … }}` / `{% … %}` delimiters bypasses the engine
  entirely, so an ordinary skill full of JSON braces is never mangled.
- `agent` and `agent_name` are built-ins — declaring either in `template_vars`
  is an error, not a silent override.
- A per-agent var with no entry for the host agent and no `default` is an error
  too. Resolution never guesses.

The rendered `SKILL.md` is what lands in the agent's skills directory; the
source stays a template.

## Subagents

Subagents are agent definitions installed into each agent's native subagents
directory (Claude Code: `~/.claude/agents/`, OpenCode: `~/.config/opencode/agent/`,
pi: `$PI_CODING_AGENT_DIR/agents` or `~/.pi/agent/agents`; agents without native
subagent support are skipped). A definition is either a
**file** (the default form — a Markdown file with frontmatter and the prompt as
body) or an **inline** entry rendered to that same format on install:

```yaml
bootstrap:
  default_subagents:
    - ./agents/reviewer.md          # file: name is the file stem ("reviewer")
    - name: planner                 # inline
      description: Plans work before execution
      tools: [Read, Grep]           # optional tool allowlist
      model: sonnet                 # optional
      prompt: |
        You are a planning specialist...
  agents:
    claude code:
      additional_subagents:
        - ./agents/claude-only.md
```

Installs are idempotent and overwrite without prompting — the project config is
the source of truth.

### Models and providers

A subagent's `model:` is resolved against the [`providers:`](#) registry at
install time and rendered in the form each host expects — Claude Code takes a
bare id, OpenCode takes `provider/id`:

| `model:` in the subagent | Claude Code | OpenCode |
| ------------------------ | ----------- | -------- |
| `haiku` (one provider serves it) | `claude-haiku-4-5` | `anthropic/claude-haiku-4-5` |
| `bedrock/opus` (provider pinned) | `anthropic.claude-opus-4-8` | `bedrock/anthropic.claude-opus-4-8` |
| `opus` (two providers serve it)  | `opus` — passed through | `opus` — passed through |

**`providers:` is optional.** With no registry — or a name it doesn't know —
the value passes through untouched and the host resolves it. Resolution also
declines to guess: a bare name matching *two* providers passes through rather
than silently binding to the wrong one.

::: tip pi: tier aliases map to taskflow roles, not the provider registry
pi subagents run under taskflow, which picks a model per **tier role**
(`settings.json.modelRoles`), not per provider. So for `Agent::Pi` a bare tier
alias in `model:` is rewritten to that role's placeholder *before*, and
independent of, any `providers:` lookup — any other value (an explicit id or
`provider/id`) passes through unchanged:

| `model:` in the subagent | Installed for pi |
| ------------------------ | ----------------- |
| `haiku`  | `"{{scout}}"`  |
| `sonnet` | `"{{builder}}"` |
| `opus`   | `"{{expert}}"` |
| `anthropic/claude-sonnet-5` (explicit id) | unchanged |
:::

#### One provider per agent

When a machine serves the same model through more than one provider, give each
agent a `default_provider` instead of qualifying every subagent. This is how you
run a personal agent and a work agent from one config:

```yaml
bootstrap:
  agents:
    claude code:
      default_provider: anthropic # personal
    opencode:
      default_provider: bedrock # work
```

A shared `model: opus` now resolves to `claude-opus-4-8` for Claude Code and
`bedrock/anthropic.claude-opus-4-8` for OpenCode. Precedence, highest first:

1. the spec's own prefix — `bedrock/opus`
2. the agent's `default_provider`
3. a unique match across the whole registry
4. pass through untouched

A `default_provider` that doesn't serve the requested model falls to (4) rather
than borrowing another provider's id — so a typo degrades to a host-side error
instead of quietly running the wrong backend.

#### Per-subagent overrides

`agents.<id>.subagent_models` pins a specific subagent's model on one agent
without touching the shared definition or the registry. The spec replaces the
subagent's frontmatter `model:` (inserted if absent) and then resolves like any
other spec — so a host-native form the registry doesn't know passes through
verbatim:

```yaml
bootstrap:
  agents:
    opencode:
      subagent_models:
        explore: corp/claude-haiku-4-5 # OpenCode-native provider/id, verbatim
```


This is what makes one config work across machines. `providers:` merges as a
whole-field override (project beats global), so leave it out of the committed
`.workmux.yaml` and let each machine's global config supply its own:

```yaml
# ~/.config/workmux/config.yaml — personal machine
providers:
  anthropic:
    limit: 200000
    models:
      - name: haiku
        id: claude-haiku-4-5
```

```yaml
# ~/.config/workmux/config.yaml — corporate machine
providers:
  bedrock:
    limit: 200000
    models:
      - name: haiku
        id: anthropic.claude-haiku-4-5
```

The shared subagent stays `model: haiku` on both.

A provider can also declare *connection* fields (`base_url`, `api_key_env`,
`npm`, `api`, `options`); `workmux setup` then renders it into each supported
agent's native provider config — see
[provider sync](./models.md#provider-sync).

## Agent settings

Harness items are *what an agent carries*. `agents.<id>.settings` is *how the
agent is configured*: an RFC 7386 merge patch applied to the agent's own
settings file by `workmux setup`.

```yaml
bootstrap:
  agents:
    pi:
      additional_plugins:
        - npm:pi-unified-exec # session-oriented exec, replaces blocking bash
      settings:
        # Built-in tools pi starts with. `bash` is omitted: the plugin above
        # provides exec_command/write_stdin instead.
        defaultTools: [read, edit, write, grep, find, ls]
```

Semantics:

- A named key replaces the current value; nested maps merge recursively; a key
  set to `null` is **deleted**.
- Every key the patch does not name survives, including ones the agent writes
  for itself (pi's `packages`, its last-seen version).
- Re-running setup with an unchanged patch rewrites nothing: drift is judged on
  the parsed value, not on formatting.
- A missing settings file is created containing the patch. A settings file that
  is not valid JSON fails that item and is left untouched.

Two caveats worth stating plainly:

- **Removing a declaration does not restore the old value.** Workmux cannot know
  what the value was before it patched, and restoring a stale one is worse than
  leaving the current one. To remove a key, set it to `null` and keep the
  declaration until the key is gone everywhere.
- **`bootstrap:` replaces wholesale across config layers.** A project config
  with its own `bootstrap.agents.pi` block shadows the global one entirely,
  settings patch included.

Supported wherever the agent keeps its config in JSON:

| Agent       | File patched                                   |
| ----------- | ---------------------------------------------- |
| pi          | `~/.pi/agent/settings.json` (`PI_CODING_AGENT_DIR`) |
| omp         | `~/.omp/agent/settings.json` (`OMP_CODING_AGENT_DIR`) |
| Claude Code | `~/.claude/settings.json` (`CLAUDE_CONFIG_DIR`) |
| Gemini CLI  | `~/.gemini/settings.json`                      |
| OpenCode    | `~/.config/opencode/opencode.json` (`OPENCODE_CONFIG`) |

Codex and Copilot CLI are **not** supported: Codex's config is TOML
(`~/.codex/config.toml`), which a JSON merge patch cannot express, and Copilot
CLI has no known global settings file. A patch declared for either is reported
as skipped rather than written somewhere guessed.

Each agent's keys are its own — `defaultTools` means nothing to Claude Code,
`autoCompact` means nothing to pi. Workmux does not validate them against any
schema; a wrong key surfaces in the agent's own error, not here.

## pi and omp injection method

pi and omp support two ways of injecting the merged prompt. Configure it under
`bootstrap.pi`:

```yaml
bootstrap:
  pi:
    injection_method: before_agent_start   # default
```

| Method               | Behavior                                                                                           |
| -------------------- | -------------------------------------------------------------------------------------------------- |
| `before_agent_start` | (default) Writes to `workmux-pre-inject.md`; injected via pi's `before_agent_start` hook. Survives cliproxy, which strips system prompts at the API level. |
| `append_system`      | Writes directly to `APPEND_SYSTEM.md` (native pi mechanism). Does **not** survive cliproxy.         |

## Full example

```yaml
bootstrap:
  default_prompt_components:
    - fff

  features:
    caveman:
      default: caveman-full
    ponytail:
      pi: git:github.com/DietrichGebert/ponytail   # https://pi.dev/packages/pi-ponytail
      default: ponytail

  agents:
    claude code:
      additional_prompt_components:
        - code-review
    pi:
      additional_plugins:
        - npm:pi-web-access
        - npm:pi-mcp-adapter
        - npm:@ff-labs/pi-fff
        - npm:@fgladisch/pi-user-select
    omp: {}   # omp uses its own spec format; add omp-format specs here when known
```

## Removing a feature

Bootstrap is declarative in both directions: dropping something from
`.workmux.yaml` and re-running `workmux setup` removes it from the machine.

The config alone cannot make that safe — an undeclared skill in
`~/.claude/skills/` might be one workmux copied there or one you wrote by hand.
So setup records what it installs in a manifest at
`$XDG_STATE_HOME/workmux/managed.json`, and **only what the manifest claims is
ever removed**:

| Section | What removal does |
|---|---|
| `skills` | deletes the installed skill directory |
| `subagents` | deletes the installed `<name>.md` |
| `plugins` | runs the agent's uninstall (`claude plugin uninstall`, `pi remove`, `omp remove`), or drops the registration from `opencode.json`, which has no uninstall command |
| `agent-hooks` | removes that exact command from the agent's hook config, and any group it emptied |

Rules worth knowing:

- **Bundled skills are never pruned.** They are always declared.
- **Only the declaring project prunes.** Manifest entries carry the project
  root, and a feature another project still declares survives — agent config
  dirs are global, so the *last* project to drop a shared skill removes it.
- **`--only` limits pruning too.** `workmux setup --only skills` never touches
  a plugin entry.
- **Removals are drift.** `workmux setup --check` reports them as `removed`
  and exits 2 without deleting anything.
- **The first run after upgrading prunes nothing.** There is no manifest yet;
  that run writes one, and removal works from the next one on.
- **A failed removal keeps its entry** so the next run retries it.
- `workmux setup --no-prune` converges without removing anything, and leaves
  the entries in place for a later run.

## See also

- [`workmux setup`](../reference/commands/setup.md) — apply the manifest
- [`workmux provision`](../reference/commands/provision.md) — org policy that can override parts of this config
- [Models](./models.md) — the unified provider/model registry
- [Skills](./skills.md) — authoring and installing skills
