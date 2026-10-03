# Development shell for workmux.
#
# Usage:
#   nix-shell          # enter a dev shell with the Rust toolchain + protobuf
#   cargo build        # build locally (proto files come from the wng-proto submodule)
#
# This is intended for *development* only. It does not package workmux; it just
# provides the toolchain needed to build it from a checked-out working tree.
#
# Make sure the proto submodule is checked out first:
#   git submodule update --init wng-proto
{
  pkgs ? import <nixpkgs> { },
}:
pkgs.mkShell {
  nativeBuildInputs = [
    pkgs.cargo
    pkgs.rustc
    pkgs.rustfmt
    pkgs.clippy
    pkgs.protobuf
    pkgs.git
  ];

  # build.rs invokes protoc via tonic-build; point it at the nixpkgs protoc and
  # expose the well-known proto include path.
  PROTOC = "${pkgs.protobuf}/bin/protoc";
  PROTOC_INCLUDE = "${pkgs.protobuf}/include";

  # Help rust-analyzer / rustc find the standard library sources.
  RUST_SRC_PATH = "${pkgs.rustPlatform.rustLibSrc}";
}
