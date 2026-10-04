---
description: Install muxix via Homebrew, pre-built binaries, Cargo, mise, or Nix
---

# Installation

## Bash YOLO

```bash
curl -fsSL https://raw.githubusercontent.com/lcensies/muxix/main/scripts/install.sh | bash
```

## Homebrew (macOS/Linux)

```bash
brew install lcensies/muxix/muxix
```

## Other methods

### Cargo

Requires Rust. Install via [rustup](https://rustup.rs/) if you don't have it.

```bash
cargo install muxix
```

### mise

```bash
mise use -g cargo:lcensies/muxix
```

### Nix

Requires [Nix with flakes enabled](https://nixos.wiki/wiki/Flakes).

```bash
nix profile install github:lcensies/muxix
```

Or try without installing:

```bash
nix run github:lcensies/muxix -- --help
```

See [Nix guide](/guide/nix) for flake integration and home-manager setup.

---

For manual installation, see [pre-built binaries](https://github.com/lcensies/muxix/releases/latest).

## Shell alias (recommended)

For faster typing, alias `muxix` to `mx`:

```bash
alias mx='muxix'
```

Add this to your `.bashrc`, `.zshrc`, or equivalent shell configuration file.

## Shell completions

To enable tab completions for commands and branch names, add the following to your shell's configuration file.

::: code-group

```bash [Bash]
# Add to ~/.bashrc
eval "$(muxix completions bash)"
```

```bash [Zsh]
# Add to ~/.zshrc
eval "$(muxix completions zsh)"
```

```bash [Fish]
# Add to ~/.config/fish/config.fish
muxix completions fish | source
```

:::
