{
  description = "Parallel development in tmux with git worktrees";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";


  outputs =
    { self, nixpkgs }:
    let
      forAllSystems =
        f:
        nixpkgs.lib.genAttrs [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ] (
          system: f nixpkgs.legacyPackages.${system}
        );
      # vendor/ also holds unrelated checkouts (Go, TypeScript). The Rust ones
      # are path and [patch] dependencies (Cargo.toml:61-63, :98), so they are
      # build inputs; discovering them beats listing them and going stale.
      rustVendorCrates = nixpkgs.lib.mapAttrsToList (name: _: ./vendor + "/${name}") (
        nixpkgs.lib.filterAttrs (
          name: type: type == "directory" && builtins.pathExists (./vendor + "/${name}/Cargo.toml")
        ) (builtins.readDir ./vendor)
      );
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          nativeBuildInputs = [
            pkgs.cargo
            pkgs.rustc
            pkgs.protobuf
            pkgs.git
          ];
          PROTOC = "${pkgs.protobuf}/bin/protoc";
        };
      });
      # A pure attrs -> config-file renderer, usable without the module system:
      #   workmux.lib.mkWorkmuxConfig { agent = "claude"; }
      lib = {
        mkWorkmuxConfig =
          settings:
          let
            pkgs = nixpkgs.legacyPackages.${builtins.currentSystem or "x86_64-linux"};
          in
          builtins.readFile ((pkgs.formats.yaml { }).generate "workmux-config.yaml" settings);
      };

      homeManagerModules.workmux =
        { config, lib, pkgs, ... }:
        import ./nix/module.nix {
          installPackage = cfg: { home.packages = [ cfg.package ]; };
          defaultPackage = self.packages.${pkgs.system}.default or null;
          mkConfigFile = file: {
            xdg.configFile."workmux/config.yaml".source = file;
          };
          # Item failures must not fail activation: a missing agent CLI is a
          # normal state, not a broken system generation.
          mkActivation = command: {
            home.activation.workmuxSetup = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
              ${command} || echo "workmux setup reported failures (activation continues)"
            '';
          };
        } { inherit config lib pkgs; };

      nixosModules.workmux =
        { config, lib, pkgs, ... }:
        import ./nix/module.nix {
          installPackage = cfg: { environment.systemPackages = [ cfg.package ]; };
          defaultPackage = self.packages.${pkgs.system}.default or null;
          mkConfigFile = file: {
            environment.etc."workmux/config.yaml".source = file;
          };
          # NixOS has no per-user activation hook here; setup is a user-level
          # action, so it stays a Home Manager concern.
          mkActivation = null;
        } { inherit config lib pkgs; };

      # Renders a fixture through the module's own generator, then makes the
      # real binary resolve it. Catches the drift that matters: the Rust config
      # schema changing under the hand-written nix options.
      checks = forAllSystems (pkgs: {
        workmux-module =
          let
            workmux = self.packages.${pkgs.system}.default;
            settings = {
              agent = "claude";
              merge_strategy = "rebase";
              include = [ "./extra.yaml" ];
              profiles.corp.agent = "codex";
              bootstrap.default_prompt_components = [ "fff" ];
              mcp.context7 = {
                command = "npx";
                args = [ "-y" "@upstash/context7-mcp" ];
              };
            };
            rendered = (pkgs.formats.yaml { }).generate "workmux-config.yaml" settings;
          in
          pkgs.runCommand "workmux-module-check" { nativeBuildInputs = [ workmux ]; } ''
            export HOME=$TMPDIR
            cd $TMPDIR
            cp ${rendered} config.yaml
            # The include is part of the fixture: resolution must expand it, not
            # merely tolerate it.
            echo 'worktree_prefix: from-include' > extra.yaml

            workmux config validate --file config.yaml --strict

            base=$(workmux config resolve --file config.yaml --format json)
            corp=$(workmux config resolve --file config.yaml --format json --profile corp)

            check() { # <label> <jq-ish grep> <haystack>
              echo "$3" | grep -q "$2" || { echo "FAIL: $1"; echo "$3"; exit 1; }
            }
            check "agent from settings"      '"agent": *"claude"'       "$base"
            check "include was expanded"     '"worktree_prefix"'        "$base"
            check "profile overrides agent"  '"agent": *"codex"'        "$corp"
            check "mcp survived rendering"   'context7'                 "$base"

            touch $out
          '';
      });

      packages = forAllSystems (pkgs: {
        default = pkgs.rustPlatform.buildRustPackage {
          pname = "workmux";
          # Cargo.toml, not the git rev: a dirty tree changed the derivation on
          # every edit, so no build was ever reused.
          version = (nixpkgs.lib.importTOML ./Cargo.toml).package.version;
          # Only what the build reads. Editing docs/ used to invalidate the
          # source hash and recompile all 421 dependencies.
          # The non-obvious entries are `include_str!` targets compiled into
          # the binary (`rg include_str src`), not just the crate sources.
          src = nixpkgs.lib.fileset.toSource {
            root = ./.;
            fileset = nixpkgs.lib.fileset.unions ([
              ./Cargo.toml
              ./Cargo.lock
              ./build.rs
              ./src
              ./resources
              ./docker
              ./README.md
              ./CHANGELOG.md
              ./.claude-plugin
              ./.codex/hooks
              ./.github/hooks
              ./.pi/extensions
              ./skills
              ./scripts
            ] ++ rustVendorCrates);
          };
          # Cargo.toml's `release` is fat LTO + codegen-units=1, so the final
          # link is single-threaded. That is wanted for CI artifacts, not for
          # a local bootstrap binary — these two are what `release-fast` sets.
          # Overriding via env rather than `buildType = "release-fast"`:
          # cargo-build-hook.sh:10 expands the profile into
          # CARGO_PROFILE_<NAME>_STRIP, and a hyphen is not a valid shell
          # identifier, so a custom profile name fails the build outright.
          CARGO_PROFILE_RELEASE_LTO = "thin";
          CARGO_PROFILE_RELEASE_CODEGEN_UNITS = "16";
          # The suite belongs to `cargo test` and CI, not to packaging.
          doCheck = false;
          cargoLock = {
            lockFile = ./Cargo.lock;
            outputHashes = {
              "crossterm-0.29.0" = "sha256-rfAaqGylDaxx3bjmofifnzSh7Hmh21BzHp5fS/w2Z6I=";
            };
          };
          nativeBuildInputs = [
            pkgs.installShellFiles
            pkgs.git
            pkgs.protobuf
          ];
          postInstall = ''
            export HOME=$TMPDIR
            installShellCompletion --cmd workmux \
              --bash <($out/bin/workmux completions bash) \
              --fish <($out/bin/workmux completions fish) \
              --zsh <($out/bin/workmux completions zsh)
          '';
        };
      });
    };
}
