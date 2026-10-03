---
description: Display the README with terminal formatting
---

# docs

Displays the README with terminal formatting. Useful for quick reference without leaving the terminal.

```bash
muxix docs
```

When run interactively, renders markdown with colors and uses a pager (`less`). When piped (e.g., to an LLM), outputs raw markdown for clean context.

## Using with AI agents

You can ask an agent to read the docs and configure muxix for you:

```
> run `muxix docs` and configure muxix so that on the left pane
  there is claude as agent, and on the right side neovim and empty
  shell on top of each other

⏺ Bash(muxix docs)
  ⎿  <p align="center">
       <picture>
     … +923 lines

⏺ Write(.muxix.yaml)
  ⎿  Wrote 9 lines to .muxix.yaml

⏺ Created .muxix.yaml with the layout:
  - Left: claude agent (focused)
  - Right top: neovim
  - Right bottom: empty shell
```
