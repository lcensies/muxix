# Shared workmux module, instantiated by both the Home Manager and NixOS
# wrappers in flake.nix.
#
# Design note: the typed options below cover the stable, frequently-used surface
# only. The workmux config schema is ~6.7k lines of Rust; mirroring all of it
# here by hand would be a second copy maintained by nobody, stale within a
# release, and its staleness would present as "nix rejects a config workmux
# accepts". Instead:
#
#   * common keys get real types, so typos and wrong types are eval errors;
#   * everything else goes through `settings`, a freeform attrset;
#   * the whole rendered file is checked at build time by the real binary
#     (`workmux config validate --strict`), which cannot drift from the schema.
#
# See the change design (D6) for the rejected alternatives.

{
  # Which config path this platform writes to, and how to install a file there.
  # Supplied by the Home Manager / NixOS wrapper.
  mkConfigFile,
  # Function producing the activation/system snippet that runs setup, or null
  # when the platform does not support one.
  mkActivation ? null,
  installPackage,
  # This flake's package for the evaluating system, used as `package`'s default
  # so `enable = true` alone is a working configuration.
  defaultPackage ? null,
}:

{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.workmux;
  format = pkgs.formats.yaml { };

  # Options with a dedicated type, in the order they appear in the config file.
  # `null` means "not set" and is dropped before rendering, so an unset option
  # never appears in the YAML as an explicit null (which workmux reads as
  # "delete this key").
  typedSettings = lib.filterAttrs (_: v: v != null) {
    inherit (cfg)
      agent
      mainBranch
      baseBranch
      worktreeDir
      mergeStrategy
      defaultProfile
      ;
  };

  # Rename the camelCase option names onto the snake_case the config uses.
  renamed =
    let
      renames = {
        mainBranch = "main_branch";
        baseBranch = "base_branch";
        worktreeDir = "worktree_dir";
        mergeStrategy = "merge_strategy";
        defaultProfile = "default_profile";
      };
    in
    lib.mapAttrs' (name: value: lib.nameValuePair (renames.${name} or name) value) typedSettings;

  structured = lib.filterAttrs (_: v: v != null && v != { } && v != [ ]) {
    include = cfg.include;
    profiles = cfg.profiles;
    bootstrap = cfg.bootstrap;
    mcp = cfg.mcp;
    providers = cfg.providers;
  };

  # Typed options win over `settings` on conflict, and the collision is
  # reported: silently dropping one of two explicit values is the kind of thing
  # someone debugs for an hour.
  conflicts = lib.intersectAttrs (renamed // structured) cfg.settings;

  merged = lib.recursiveUpdate cfg.settings (renamed // structured);

  rendered = format.generate "workmux-config.yaml" merged;

  # Build-time validation: the real binary parses the rendered file, so an
  # unrecognized key fails the build rather than surfacing at activation or at
  # the user's next `workmux` invocation.
  validated =
    if cfg.validateConfig then
      pkgs.runCommand "workmux-config-validated.yaml"
        {
          nativeBuildInputs = [ cfg.package ];
        }
        ''
          export HOME=$TMPDIR
          workmux config validate --file ${rendered} --strict
          cp ${rendered} $out
        ''
    else
      rendered;

  profileArgs = lib.optionals (cfg.defaultProfile != null) [
    "--profile"
    cfg.defaultProfile
  ];
  sectionArgs = lib.optionals (cfg.applySections != [ ]) [
    "--only"
    (lib.concatStringsSep "," cfg.applySections)
  ];
  setupCommand = lib.concatStringsSep " " (
    [ "${cfg.package}/bin/workmux" "setup" "--non-interactive" ] ++ profileArgs ++ sectionArgs
  );
in
{
  options.programs.workmux = {
    enable = lib.mkEnableOption "workmux, the git-worktree and tmux harness";

    package = lib.mkOption {
      type = lib.types.package;
      default = defaultPackage;
      defaultText = lib.literalExpression "workmux.packages.\${system}.default";
      description = "The workmux package to install and to validate the config with.";
    };

    # --- typed scalars ------------------------------------------------------

    agent = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "claude";
      description = "Default coding agent to launch in new worktrees.";
    };

    mainBranch = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Primary branch to merge into. Auto-detected when unset.";
    };

    baseBranch = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Branch new worktrees are created from.";
    };

    worktreeDir = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Directory template for new worktrees.";
    };

    mergeStrategy = lib.mkOption {
      type = lib.types.nullOr (lib.types.enum [ "rebase" "squash" "merge" ]);
      default = null;
      description = "How `workmux merge` integrates a branch.";
    };

    # --- composition --------------------------------------------------------

    include = lib.mkOption {
      type = lib.types.listOf (lib.types.either lib.types.str (lib.types.attrsOf lib.types.anything));
      default = [ ];
      example = [ "~/.config/workmux/corp.yaml" ];
      description = ''
        Config files merged beneath this one, in declaration order.

        Rendered into the YAML rather than inlined, so workmux resolves them at
        load time. That keeps an include pointing at a file the user edits
        working without a nix rebuild.
      '';
    };

    profiles = lib.mkOption {
      type = lib.types.attrsOf format.type;
      default = { };
      example = lib.literalExpression ''
        {
          corp.agent = "codex";
          personal.agent = "claude";
        }
      '';
      description = ''
        Named partial configs, selected at runtime with `--profile`,
        `WORKMUX_PROFILE`, or `defaultProfile`.

        Rendered into the config's `profiles:` block, so selection stays a
        runtime decision rather than requiring a rebuild per profile.
      '';
    };

    defaultProfile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = ''
        Profile(s) applied when nothing higher-precedence selects one.
        Comma-separated for several. Also used by `applyOnActivation`.
      '';
    };

    # --- harness ------------------------------------------------------------

    bootstrap = lib.mkOption {
      type = format.type;
      default = { };
      example = lib.literalExpression ''
        {
          default_skills = [ "./skills/workmux" ];
          default_prompt_components = [ "fff" ];
          theme.default = "catppuccin";
        }
      '';
      description = "Agent bootstrap: skills, subagents, plugins, prompt components, theme.";
    };

    mcp = lib.mkOption {
      type = lib.types.attrsOf format.type;
      default = { };
      example = lib.literalExpression ''
        { context7 = { command = "npx"; args = [ "-y" "@upstash/context7-mcp" ]; }; }
      '';
      description = "MCP servers exposed to agents.";
    };

    providers = lib.mkOption {
      type = format.type;
      default = { };
      description = "Model provider registry.";
    };

    settings = lib.mkOption {
      type = format.type;
      default = { };
      example = lib.literalExpression ''
        {
          panes = [ { command = "<agent>"; focus = true; } ];
          orchestrate.merge.auto_merge = true;
        }
      '';
      description = ''
        Any config key without a dedicated option above, rendered verbatim.

        Not a typo escape hatch: the rendered file is validated by the workmux
        binary at build time, so an unrecognized key still fails the build
        (unless `validateConfig` is off).
      '';
    };

    # --- behavior -----------------------------------------------------------

    validateConfig = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = ''
        Run `workmux config validate --strict` on the rendered file at build
        time. Turn off when cross-building to a system whose workmux binary
        cannot execute on the builder.
      '';
    };

    applyOnActivation = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = ''
        Run `workmux setup --non-interactive` during activation, applying the
        declared harness to whichever agent CLIs are installed.

        Off by default: it writes into agent config directories workmux does not
        own, which is a side effect an activation should opt into rather than
        inherit. Item-level failures are logged and never fail activation.
      '';
    };

    applySections = lib.mkOption {
      type = lib.types.listOf (
        lib.types.enum [ "hooks" "skills" "subagents" "plugins" "prompts" "theme" "mcp" ]
      );
      default = [ ];
      example = [ "hooks" "mcp" ];
      description = "Restrict `applyOnActivation` to these sections. Empty means all.";
    };
  };

  config = lib.mkIf cfg.enable (
    lib.mkMerge [
      {
        warnings = lib.optional (conflicts != { }) ''
          programs.workmux: these keys are set both as typed options and under
          `settings`; the typed option wins: ${lib.concatStringsSep ", " (lib.attrNames conflicts)}
        '';
      }
      (installPackage cfg)
      (mkConfigFile validated)
      (lib.optionalAttrs (mkActivation != null) (
        lib.mkIf cfg.applyOnActivation (mkActivation setupCommand)
      ))
    ]
  );
}
