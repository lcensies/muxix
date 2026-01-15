{
  description = "Parallel development in tmux with git worktrees";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      version = "0.1.92";

      platforms = {
        x86_64-linux = {
          name = "linux-amd64";
          hash = "sha256:05d1798684fcec6bcb3db6f63426434a3810887cbc2cb8e868799ace1ed248bd";
        };
        aarch64-linux = {
          name = "linux-arm64";
          hash = "sha256:bd37c0d972f695a5cc71faebc1a0215fefc9ef15cbbf4fddd691642d719f4e03";
        };
        x86_64-darwin = {
          name = "darwin-amd64";
          hash = "sha256:d1aa680ffece309f5750c3a90a2ed50427b0e40bf5a4b3e420927249a9676cfe";
        };
        aarch64-darwin = {
          name = "darwin-arm64";
          hash = "sha256:075447b9edef44dbc93eaa473af02474c3e102cedf0eb270b6b19162d64b6981";
        };
      };

      forAllSystems = f: nixpkgs.lib.genAttrs (builtins.attrNames platforms)
        (system: f system nixpkgs.legacyPackages.${system});
    in {
      packages = forAllSystems (system: pkgs: let
        platform = platforms.${system};
      in {
        default = pkgs.stdenv.mkDerivation {
          pname = "workmux";
          inherit version;

          src = pkgs.fetchurl {
            url = "https://github.com/raine/workmux/releases/download/v${version}/workmux-${platform.name}.tar.gz";
            hash = platform.hash;
          };

          sourceRoot = ".";
          dontConfigure = true;
          dontBuild = true;

          installPhase = ''
            install -Dm755 workmux $out/bin/workmux
          '';
        };
      });
    };
}
