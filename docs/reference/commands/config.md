---
description: Edit, locate, or view the global muxix configuration file
---

# config

Manage the global muxix configuration file (`~/.config/muxix/config.yaml`).

## config edit

Open the global configuration file in your preferred editor.

```bash
muxix config edit
```

Uses `$VISUAL`, then `$EDITOR`, then falls back to `vi`. If the configuration file does not exist yet, it is created with commented-out defaults before opening.

## config path

Print the path to the global configuration file. Useful for scripting.

```bash
muxix config path
# Output: /home/user/.config/muxix/config.yaml
```

## config reference

Print the default configuration file with all options documented. Useful for discovering available options or piping to an AI agent for context.

```bash
muxix config reference
```

## Examples

```bash
# Edit global config
muxix config edit

# Use a specific editor
EDITOR=nano muxix config edit

# Print the config path (for use in scripts)
cat "$(muxix config path)"

# Print the default config reference
muxix config reference
```

## See also

- [Configuration guide](/guide/configuration) for all available options
- [`init`](./init) to generate a project-level `.muxix.yaml`
