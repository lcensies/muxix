---
description: Installing and configuring muxix with Nix, Home Manager, and NixOS
---

# Nix

Requires [Nix with flakes enabled](https://nixos.wiki/wiki/Flakes).

## Quick start

Run muxix without installing:

```bash
nix run github:lcensies/muxix -- --help
```

## Flake input

```nix
inputs.muxix.url = "github:lcensies/muxix";
```

## Declaring the harness

The module turns your muxix configuration into a rendered `config.yaml`. The
whole harness — agents, skills, subagents, MCP servers, prompt components — is
declared in Nix and materialized on activation.

### Home Manager

```nix
{ inputs, pkgs, ... }:

{
  imports = [ inputs.muxix.homeManagerModules.muxix ];

  programs.muxix = {
    enable = true;

    agent = "claude";
    mergeStrategy = "rebase";

    bootstrap = {
      default_skills = [ "./skills/muxix" ];
      default_prompt_components = [ "fff" "slim-comments" ];
      theme.default = "catppuccin";
    };

    mcp.context7 = {
      command = "npx";
      args = [ "-y" "@upstash/context7-mcp" ];
    };

    # Anything without a dedicated option goes here verbatim.
    settings = {
      panes = [ { command = "<agent>"; focus = true; } ];
    };
  };
}
```

This writes `~/.config/muxix/config.yaml` and installs the package.

### NixOS

```nix
{ inputs, ... }:

{
  imports = [ inputs.muxix.nixosModules.muxix ];
  programs.muxix.enable = true;
}
```

The NixOS module writes `/etc/muxix/config.yaml` and installs the package
system-wide. It has no activation step — applying a harness writes into per-user
agent config directories, so that stays a Home Manager concern.

## Typed options and `settings`

Common keys have real options — `agent`, `mainBranch`, `baseBranch`,
`worktreeDir`, `mergeStrategy`, `include`, `profiles`, `defaultProfile`,
`bootstrap`, `mcp`, `providers`. A typo or a wrong type in those is a Nix
evaluation error.

Everything else goes through `settings`, which is free-form. That is not a hole:
the rendered file is checked at build time by the muxix binary itself, so an
unrecognized key fails the build too — just later, with a different message.

::: tip Why not type the whole schema
The muxix config schema is thousands of lines of Rust. A hand-written Nix
mirror of it would be a second copy maintained by nobody, stale within a
release, and its staleness would show up as "Nix rejects a config muxix
accepts". Validating the rendered file with the real binary cannot drift.
:::

## Build-time validation

`muxix config validate --strict` runs against the rendered file during the
build:

```nix
programs.muxix.validateConfig = false;  # default: true
```

Turn it off only when cross-building to a system whose muxix binary cannot
execute on the builder.

## Profiles

Profiles are rendered into the config's `profiles:` block, so selecting one
stays a runtime decision — no rebuild per profile.

```nix
programs.muxix = {
  defaultProfile = "personal";

  profiles = {
    personal.agent = "claude";
    corp = {
      agent = "codex";
      bootstrap.default_prompt_components = [ "no-coauthor" ];
    };
  };
};
```

```bash
muxix --profile corp add my-branch
MUXIX_PROFILE=corp muxix add my-branch
```

See [Profiles](/guide/profiles) for the precedence rules.

## Includes

`include` is rendered as a path, not inlined, so muxix resolves it at load
time. An include pointing at a file you edit by hand keeps working without a
rebuild — which is the point of using one rather than folding the content into
Nix.

```nix
programs.muxix.include = [ "~/.config/muxix/corp.yaml" ];
```

## Applying the harness on activation

Writing `config.yaml` does not install skills, hooks, or MCP servers. That is
what `muxix setup` does, and the module can run it for you:

```nix
programs.muxix = {
  applyOnActivation = true;              # default: false
  applySections = [ "hooks" "mcp" ];     # default: all sections
};
```

Off by default because it writes into agent config directories muxix does not
own — a side effect an activation should opt into rather than inherit.
Item-level failures (an agent CLI that is not installed, a plugin that fails to
fetch) are logged and never fail activation.

Without it, run `muxix setup` yourself:

```bash
muxix setup --non-interactive
muxix setup --check           # exit 2 if the machine has drifted
```

## Rendering a config without the module system

`lib.mkMuxixConfig` is a plain `attrs -> string` renderer, useful for
generating a config outside Home Manager or NixOS:

```nix
inputs.muxix.lib.mkMuxixConfig {
  agent = "claude";
  merge_strategy = "rebase";
}
```

Note it takes the config's own snake_case keys, not the module's camelCase
options.

## Checks

`nix flake check` renders a fixture through the module's generator and makes the
real binary resolve it, so a change to the Rust config schema that the Nix
options do not follow fails there rather than on someone's machine.

## Shell completions

The flake installs completions for Bash, Zsh, and Fish. Do not add the manual
`eval "$(muxix completions ...)"` lines from the
[Installation](/guide/installation#shell-completions) guide.
