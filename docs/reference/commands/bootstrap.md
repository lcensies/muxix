---
description: Declare what agent harnesses contain — plugins, skills, subagents, prompt components — and apply it
---

# bootstrap

Edits the `bootstrap:` block of a workmux config file and applies it with
[`workmux setup`](./setup.md), so what an agent has installed is always what the
config declares. Installing through an agent's own installer instead leaves the
item undeclared, untracked in the managed manifest, and never pruned.

```bash
workmux bootstrap plugin   add <SPEC>...
workmux bootstrap skill    add <PATH|URL>...
workmux bootstrap subagent add <PATH>...
workmux bootstrap prompt   add <NAME>...
```

Each kind also takes `rm` (alias `remove`). Removing a declaration is what
uninstalls it: setup prunes what the manifest says workmux installed, unless
another project still declares it.

## Options

| Flag             | Effect                                                                                   |
| ---------------- | ---------------------------------------------------------------------------------------- |
| `--agent <NAME>` | Write to `bootstrap.agents.<name>.additional_*` instead of the shared `default_*` list     |
| `--global`       | Write the global config                                                                    |
| `--project`      | Write the project `.workmux.yaml` (fails when there is none)                               |
| `--no-sync`      | Declare only; do not run setup                                                             |

Without `--global` or `--project`, the edit goes to the project `.workmux.yaml`
when one is discoverable and the global config otherwise. The file written is
always printed.

Agent names are the usual config keys: `claude`, `codex`, `copilot`, `gemini`,
`opencode`, `pi`, `omp`.

## bootstrap list

```bash
workmux bootstrap list
workmux bootstrap list --agent pi
```

Prints what each detected agent resolves to from the effective config, tagging
each item with its origin: `[shared]`, `[feature]`, or the agent's own
`additional_*` list.

## bootstrap sync

```bash
workmux bootstrap sync
```

A full non-interactive `workmux setup`. Use it after hand-editing a config or
after `--no-sync`.

## Examples

```bash
# a pi extension, for pi only, declared in this repo's config
workmux bootstrap plugin add git:github.com/DietrichGebert/ponytail --agent pi

# a local skill for every agent, on this machine
workmux bootstrap skill add ./skills/auto-git --global

# stop using it everywhere (uninstalls on the same run)
workmux bootstrap skill rm ./skills/auto-git --global
```

## Notes

- `add` is idempotent: a spec already declared is reported and nothing is
  written.
- `rm` fails when the spec is not declared in the target list.
- Comments elsewhere in the config survive the edit; comments inside the edited
  list do not, because that list is re-rendered.
- A failing sync does not roll back the edit — the declaration stands and
  `workmux bootstrap sync` retries it.
- The bundled `agent-packages` skill teaches coding agents to use these commands
  instead of their own installers.
