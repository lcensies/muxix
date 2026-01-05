# Installation

## Homebrew (macOS/Linux)

```bash
brew install raine/workmux/workmux
```

## Pre-built binaries

Download the [latest release](https://github.com/raine/workmux/releases/latest) for your platform:

| Platform              | Download                                                                                                             |
| --------------------- | -------------------------------------------------------------------------------------------------------------------- |
| Linux (x64)           | [workmux-linux-amd64.tar.gz](https://github.com/raine/workmux/releases/latest/download/workmux-linux-amd64.tar.gz)   |
| Linux (ARM64)         | [workmux-linux-arm64.tar.gz](https://github.com/raine/workmux/releases/latest/download/workmux-linux-arm64.tar.gz)   |
| macOS (Intel)         | [workmux-darwin-amd64.tar.gz](https://github.com/raine/workmux/releases/latest/download/workmux-darwin-amd64.tar.gz) |
| macOS (Apple Silicon) | [workmux-darwin-arm64.tar.gz](https://github.com/raine/workmux/releases/latest/download/workmux-darwin-arm64.tar.gz) |

Extract and install:

```bash
tar xzf workmux-*.tar.gz
sudo mv workmux /usr/local/bin/
```

## Cargo

Requires Rust. Install via [rustup](https://rustup.rs/) if you don't have it.

```bash
cargo install workmux
```

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
