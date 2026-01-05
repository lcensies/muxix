# Tips & Tricks

## Nerdfont window prefix

If you have a [Nerd Font](https://www.nerdfonts.com/) installed (fonts patched with icons for developers), you can use the git branch icon as your window prefix for a cleaner look:

```yaml
# ~/.config/workmux/config.yaml
window_prefix: "\uf418 "
```

<div style="display: flex; justify-content: center; margin: 1.5rem 0;">
  <img src="/nerdfont-prefix.webp" alt="nerdfont window prefix" style="border-radius: 4px;">
</div>

## Using direnv

If your project uses [direnv](https://direnv.net/) for environment management, you can configure workmux to automatically set it up in new worktrees:

```yaml
# .workmux.yaml
post_create:
  - direnv allow

files:
  symlink:
    - .envrc
```

## Using workmux docs

workmux includes a `docs` command that displays the full README with terminal formatting. Useful for quick reference without leaving the terminal.

```bash
workmux docs
```

When run interactively, renders markdown with colors and uses a pager (`less`). When piped (e.g., to an LLM), outputs raw markdown for clean context.

### Using with AI agents

You can ask an agent to read the docs and configure workmux for you:

```
> run `workmux docs` and configure workmux so that on the left pane
  there is claude as agent, and on the right side neovim and empty
  shell on top of each other

⏺ Bash(workmux docs)
  ⎿  <p align="center">
       <picture>
     … +923 lines

⏺ Write(.workmux.yaml)
  ⎿  Wrote 9 lines to .workmux.yaml

⏺ Created .workmux.yaml with the layout:
  - Left: claude agent (focused)
  - Right top: neovim
  - Right bottom: empty shell
```
