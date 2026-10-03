---
description: Installing and configuring workmux with Nix, Home Manager, and NixOS
---

# Nix

Requires [Nix with flakes enabled](https://nixos.wiki/wiki/Flakes).

## Quick start

Run workmux without installing:

```bash
nix run github:raine/workmux -- --help
```

## Flake input

```nix
inputs.workmux.url = "github:raine/workmux";
```

## Declaring the harness

The module turns your workmux configuration into a rendered `config.yaml`. The
whole harness — agents, skills, subagents, MCP servers, prompt components — is
declared in Nix and materialized on activation.

### Home Manager

```nix
{ inputs, pkgs, ... }:

{
  imports = [ inputs.workmux.homeManagerModules.workmux ];

  programs.workmux = {
    enable = true;

    agent = "claude";
    mergeStrategy = "rebase";

    bootstrap = {
      default_skills = [ "./skills/workmux" ];
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

This writes `~/.config/workmux/config.yaml` and installs the package.

### NixOS

```nix
{ inputs, ... }:

{
  imports = [ inputs.workmux.nixosModules.workmux ];
  programs.workmux.enable = true;
}
```

The NixOS module writes `/etc/workmux/config.yaml` and installs the package
system-wide. It has no activation step — applying a harness writes into per-user
agent config directories, so that stays a Home Manager concern.

## Typed options and `settings`

Common keys have real options — `agent`, `mainBranch`, `baseBranch`,
`worktreeDir`, `mergeStrategy`, `include`, `profiles`, `defaultProfile`,
`bootstrap`, `mcp`, `providers`. A typo or a wrong type in those is a Nix
evaluation error.

Everything else goes through `settings`, which is free-form. That is not a hole:
the rendered file is checked at build time by the workmux binary itself, so an
unrecognized key fails the build too — just later, with a different message.

::: tip Why not type the whole schema
The workmux config schema is thousands of lines of Rust. A hand-written Nix
mirror of it would be a second copy maintained by nobody, stale within a
release, and its staleness would show up as "Nix rejects a config workmux
accepts". Validating the rendered file with the real binary cannot drift.
:::

## Build-time validation

`workmux config validate --strict` runs against the rendered file during the
build:

```nix
programs.workmux.validateConfig = false;  # default: true
```

Turn it off only when cross-building to a system whose workmux binary cannot
execute on the builder.

## Profiles

Profiles are rendered into the config's `profiles:` block, so selecting one
stays a runtime decision — no rebuild per profile.

```nix
programs.workmux = {
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
workmux --profile corp add my-branch
WORKMUX_PROFILE=corp workmux add my-branch
```

See [Profiles](/guide/profiles) for the precedence rules.

## Includes

`include` is rendered as a path, not inlined, so workmux resolves it at load
time. An include pointing at a file you edit by hand keeps working without a
rebuild — which is the point of using one rather than folding the content into
Nix.

```nix
programs.workmux.include = [ "~/.config/workmux/corp.yaml" ];
```

## Applying the harness on activation

Writing `config.yaml` does not install skills, hooks, or MCP servers. That is
what `workmux setup` does, and the module can run it for you:

```nix
programs.workmux = {
  applyOnActivation = true;              # default: false
  applySections = [ "hooks" "mcp" ];     # default: all sections
};
```

Off by default because it writes into agent config directories workmux does not
own — a side effect an activation should opt into rather than inherit.
Item-level failures (an agent CLI that is not installed, a plugin that fails to
fetch) are logged and never fail activation.

Without it, run `workmux setup` yourself:

```bash
workmux setup --non-interactive
workmux setup --check           # exit 2 if the machine has drifted
```

## Rendering a config without the module system

`lib.mkWorkmuxConfig` is a plain `attrs -> string` renderer, useful for
generating a config outside Home Manager or NixOS:

```nix
inputs.workmux.lib.mkWorkmuxConfig {
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
`eval "$(workmux completions ...)"` lines from the
[Installation](/guide/installation#shell-completions) guide.
