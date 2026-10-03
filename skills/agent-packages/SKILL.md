---
name: agent-packages
description: Install, remove, or list an agent's own harness items — plugins, extensions, packages, skills, subagents, prompt components — through muxix instead of the agent's native installer. Use whenever the user asks to add or remove a package/plugin/extension/skill for the coding agent itself, or asks what the agent currently has installed.
allowed-tools: Bash
---

Harness items belong in muxix config. `muxix bootstrap` writes the
declaration and applies it in one step.

## Hard rule

Do **not** run an agent's native installer (`pi install`, `pi remove`, `omp
install`, an agent's plugin-add command) and do **not** hand-edit
`~/.pi/agent/settings.json`, `~/.claude/settings.json`, or a skills directory.

Anything installed that way is invisible to muxix: absent from config, absent
from the managed manifest, never pruned, and gone the next time the harness is
provisioned from config. Uninstalling it later is also manual.

## Commands

```bash
muxix bootstrap plugin   add <SPEC>...   # agent plugins / extensions / packages
muxix bootstrap skill    add <PATH|URL>...
muxix bootstrap subagent add <PATH>...
muxix bootstrap prompt   add <NAME>...   # prompt components
```

`rm` instead of `add` removes a declaration; the next sync uninstalls the item
(muxix knows it installed it).

```bash
muxix bootstrap list              # what each detected agent resolves to, with origin
muxix bootstrap list --agent pi
muxix bootstrap sync              # apply declared config (full `muxix setup`)
```

## Which flags

- `--agent <name>` — for this agent only (`pi`, `claude`, `codex`, `copilot`,
  `gemini`, `opencode`, `omp`). Use it whenever the spec is agent-specific,
  which a plugin spec almost always is.
- no `--agent` — shared across every agent. Correct for a local skill path or a
  prompt component that every agent should carry.
- `--global` — write the user's global config instead of the project's
  `.muxix.yaml`. Use for something the user wants on this machine everywhere;
  the default (project config when one exists) pins it to the repo.
- `--no-sync` — declare without installing. Only when the user asks to stage a
  change.

## Pick the target before running

Ask yourself, in order:

1. Is this specific to one agent? → `--agent <that agent>`.
2. Should it follow the user everywhere, or belong to this repo? → `--global` or
   the default.

If the user says "install X for you" while you are the running agent, that means
`--agent <yourself>`.

## A skill or MCP server that needs a CLI

Declare it on the entry, never inside the skill:

```yaml
- path: ./skills/x
  requires:
    npm: ["some-cli@1.2.3"]   # muxix installs it (pinned) and prunes it later
    bin: [python3]            # muxix only asserts it is on PATH
```

No `npx -y` in a skill script and no `npm i -g` by hand: both are undeclared,
unpinned installs the harness cannot see or remove. A `bin:` reported
`[failed]` is the user's to provide (on Nix: `home.packages`) — say so and
stop. `muxix bootstrap` has no flag for `requires` yet; edit the entry in
the config file the command prints.

## Global config owned by Nix

When `~/.config/muxix/config.yaml` is a read-only store symlink (Home
Manager / NixOS `programs.muxix`), `--global` fails with
`... is read-only (managed by Nix/Home Manager?)`. Do not `chmod`, unlink, or
write a sibling file. Either:

- the change belongs to this repo → drop `--global`, use project config; or
- it must be machine-wide → declare it in the nix source and rebuild. If a
  `harness-nix` skill is installed, follow it; otherwise tell the user which
  file is read-only and stop.

## Examples

```bash
# a pi extension, just for pi, in this repo's config
muxix bootstrap plugin add git:github.com/DietrichGebert/ponytail --agent pi

# a local skill every agent should have, on this machine
muxix bootstrap skill add ./skills/auto-git --global

# undo it
muxix bootstrap skill rm ./skills/auto-git --global
```

## Reporting back

Say which file changed (the command prints it) and whether it was applied. If
the sync step fails, the declaration still stands — report the failure and
suggest `muxix bootstrap sync` once the cause is fixed. Do not fall back to
the native installer.
