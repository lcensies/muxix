---
description: Install workmux via Homebrew, pre-built binaries, Cargo, or Nix
---

# Installation

## Bash YOLO

```bash
curl -fsSL https://raw.githubusercontent.com/raine/workmux/main/scripts/install.sh | bash
```

## Homebrew (macOS/Linux)

```bash
brew install raine/workmux/workmux
```

## Cargo

Requires Rust. Install via [rustup](https://rustup.rs/) if you don't have it.

```bash
cargo install workmux
```

## Nix

Requires [Nix with flakes enabled](https://nixos.wiki/wiki/Flakes).

```bash
nix profile install github:raine/workmux
```

Or try without installing:

```bash
nix run github:raine/workmux -- --help
```

See [Nix guide](/guide/nix) for flake integration and home-manager setup.

---

For manual installation, see [pre-built binaries](https://github.com/raine/workmux/releases/latest).

## Shell alias (recommended)

For faster typing, alias `workmux` to `wm`:

```bash
alias wm='workmux'
```

Add this to your `.bashrc`, `.zshrc`, or equivalent shell configuration file.

## Shell completions

To enable tab completions for commands and branch names, add the following to your shell's configuration file.

::: code-group

```bash [Bash]
# Add to ~/.bashrc
eval "$(workmux completions bash)"
```

```bash [Zsh]
# Add to ~/.zshrc
eval "$(workmux completions zsh)"
```

```bash [Fish]
# Add to ~/.config/fish/config.fish
workmux completions fish | source
```

:::
