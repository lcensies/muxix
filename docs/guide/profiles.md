---
description: Composing workmux configuration with includes, profiles, and layers
---

# Profiles and includes

One config file rarely fits every situation. A corporate laptop and a personal
one want different agents; a Python project and a Rust one want different
skills. Workmux composes configuration from layers rather than asking you to
maintain a file per case.

## The layer order

The effective config is built by merging layers, each overriding the one before:

1. includes of the global config (depth-first, in declaration order)
2. the global config's own keys — `~/.config/workmux/config.yaml`
3. includes of the project config
4. the project config's own keys — `.workmux.yaml`
5. **org policy defaults**, when a provisioning policy is cached
6. selected **profiles**, left to right
7. CLI flags
8. **org policy locks**, above everything

Policy sits at both ends deliberately. A *default* is a suggestion for a machine
that has not decided, so it must not beat a project's explicit choice. A *lock*
is the one thing an organization can rely on, so it wins outright — and says so.
See [provision](/reference/commands/provision).

To see the result, and where each key came from:

```bash
workmux config resolve
workmux config resolve --explain
workmux config resolve --format json --profile corp
```

`--explain` annotates every key with the layer that set it:

```yaml
merge_strategy: rebase  # from ~/.config/workmux/base.yaml
agent: codex  # from profile `corp` (--profile)
```

## Includes

`include:` merges other config files *beneath* the file that names them, so the
including file always wins.

```yaml
# ~/.config/workmux/config.yaml
include:
  - ./base.yaml
  - ~/.config/workmux/corp.yaml
  - https://config.corp.example.com/workmux.yaml
  - { path: ./optional.yaml, optional: true }

agent: claude   # wins over anything the includes set
```

Entries resolve relative to the file that declares them. Includes may nest;
their own includes rank below them.

- A cycle is an error naming the full path, not a hang.
- Nesting is capped at 16 levels.
- A missing include is an error unless marked `optional: true`.
- Remote includes must be `https://`. They are cached under the workmux cache
  directory; if a fetch fails and a cached copy exists, the copy is used and a
  warning is printed.

A config that declares no `include:` does no extra filesystem or network work —
config loading runs on every workmux invocation, so this stays off the hot path.

## Profiles

A profile is a named partial config, applied on top of everything below it.

```yaml
agent: claude
default_profile: personal

profiles:
  personal:
    agent: claude
  corp:
    agent: codex
    bootstrap:
      default_prompt_components: [no-coauthor]
  python:
    files:
      symlink: [.venv]
```

Selection, in decreasing precedence:

| Source | Example |
| --- | --- |
| `--profile` | `workmux --profile corp add feat` |
| `WORKMUX_PROFILE` | `WORKMUX_PROFILE=corp workmux add feat` |
| `default_profile:` | applied when neither of the above is set |

A higher-precedence source **replaces** the list from lower ones rather than
adding to it. Several profiles apply left to right:

```bash
workmux --profile corp,python add feat   # python wins on conflicts
```

An explicit empty value turns profiles off for one command, including a
configured `default_profile`:

```bash
workmux --profile "" config resolve
```

Profiles may be declared anywhere in the layer stack — an include or the global
config can define one a project selects. They may not nest, and may not declare
`include:`; composition already has a mechanism that operates on files, where a
reader can follow it.

## Merge rules

The same rules apply between any two layers:

| Shape in the overriding layer | Result |
| --- | --- |
| scalar or enum | replaces |
| mapping | deep-merges |
| sequence | replaces |
| `+key:` sequence | appends to the inherited list |
| `key: null` | removes the inherited key |

```yaml
# global
pre_merge: [just check]
mcp:
  context7: { command: npx, args: [-y, "@upstash/context7-mcp"] }

# project
+pre_merge: [just test]      # -> [just check, just test]
mcp:
  context7: null             # -> removed
```

Lists also support the older `<global>` placeholder, which expands to the
inherited list at that position:

```yaml
pre_merge: ["<global>", just test]
```

A few keys deliberately replace wholesale rather than deep-merging —
`bootstrap`, `providers`, `theme.custom`, `sidebar.templates` — because blending
two of them produces a config neither layer asked for. Entries under `mcp`,
`layouts`, `agent_defs`, `prompt_defs`, and `sidebar.agent_icons` replace per
entry rather than merging field by field.

## Keys a project config cannot set

Some keys are security-sensitive: a repository you clone must not be able to set
them by shipping a `.workmux.yaml`. They are ignored in project configs, with a
warning, and only take effect from the global config:

`agents`, `provision`, `auto_name.command`, `sandbox.env`,
`sandbox.env_passthrough`, `sandbox.host_commands`, `sandbox.extra_mounts`,
`sandbox.rpc_host`, `sandbox.agent_config_dir`, `sandbox.network`,
`sandbox.container.devices`, `sandbox.container.group_add`,
`sandbox.container.excluded_files`,
`sandbox.dangerously_allow_unsandboxed_host_exec`.

This applies transitively: an include pulled in by a project config, or a
profile declared in one, carries the project's trust level — a repo cannot
launder a global-only key through another file.

## Secrets

Values may name a secret rather than contain one:

```yaml
provision:
  token: ${env:MY_ORG_TOKEN}
  # or
  token_path: ${file:~/.secrets/provision-token}
```

These expand at the point of use, never during resolution, so a resolved config
— what `config resolve` prints, what a Nix module renders into the store — never
holds secret material. A `${file:...}` source must not be readable by group or
others.

## Validating

```bash
workmux config validate                       # the effective config
workmux config validate --file rendered.yaml  # one file, on its own
workmux config validate --strict              # unrecognized keys are errors
```

`--file` judges a file by itself, ignoring the ambient global and project
configs, so a generated config validates the same way on any machine. Every
declared profile is validated in turn — a profile that only breaks when selected
is a trap that surfaces on someone else's machine.

## Agent config profiles

Distinct from the *config* profiles above: `agent_profiles:` declares named
overlays of an **agent's own config directory** (e.g. `~/.pi/agent/`), selected
at launch with `workmux exec --profile <name> <agent>`. The base dir is also
the default profile; a named profile layers on top of it and never mutates it.

Two mechanisms compose, per profile:

1. **File overlay** (user-authored): anything under
   `~/.config/workmux/agent-profiles/<name>/` shadows the same path in base,
   file by file. Setup only reads this tree.
2. **Declarative deltas** (workmux.yaml-authored): per-agent adds/excludes that
   `workmux setup` materializes into generated files in the derived overlay at
   `~/.local/state/workmux/agent-profiles/<name>/<agent>/`.

```yaml
agent_profiles:
  corp:
    description: pi via corp LiteLLM proxy
    agents:
      pi:
        additional_plugins:
          - ~/dotfiles/pi/pi-provider-litellm
        exclude_plugins:            # base packages entries, exact match
          - npm:pi-cliproxyapi
        additional_skills: []       # source paths, linked at skills/<basename>
        exclude_skills: []          # installed skill dir names
        additional_prompt_components: []
        exclude_prompt_components: []
        exclude_features: []        # drops the feature's plugin and/or prompt component
        exclude_paths: []           # agent-dir-relative files/dirs, the raw escape hatch
        settings:                   # RFC 7386 merge patch; null deletes a key
          defaultProvider: litellm
```

Semantics worth knowing:

- **Generated settings derive from current base** on every `workmux setup`:
  base `packages` minus excludes, plus additions, then the `settings` patch.
  Installing a plugin into base later flows into every profile automatically.
- **Added path specs are absolutized** (`~/` → home, relative → base agent
  dir); `npm:`/`git:` specs pass verbatim but are *not* fetched into overlays —
  profile additions should be local checkouts, or packages base already has.
- **Prompt components re-render** into the overlay's prompt file when prompt
  deltas are declared; otherwise the base file stays linked.
- **The file overlay wins** over generated content: a hand-authored
  `settings.json` in the profile source tree shadows the deltas (setup warns).
- **Excludes that match nothing warn** instead of failing, so stale excludes
  never wedge setup.
- **Runtime writes are ephemeral** in generated files: pi updating its own
  settings inside a profile writes to the derived tree, which the next rebuild
  discards. Persistent preferences belong in base or the `settings` patch.
- Delta generation is implemented for **pi only**; declaring deltas for another
  agent skips that agent's overlay (visibly) rather than building it wrong.
