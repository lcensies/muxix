---
description: Edit or locate the global workmux configuration file
---

# config

Manage the global workmux configuration file (`~/.config/workmux/config.yaml`).

## config edit

Open the global configuration file in your preferred editor.

```bash
workmux config edit
```

Uses `$VISUAL`, then `$EDITOR`, then falls back to `vi`. If the configuration file does not exist yet, it is created with commented-out defaults before opening.

## config path

Print the path to the global configuration file. Useful for scripting.

```bash
workmux config path
# Output: /home/user/.config/workmux/config.yaml
```

## Examples

```bash
# Edit global config
workmux config edit

# Use a specific editor
EDITOR=nano workmux config edit

# Print the config path (for use in scripts)
cat "$(workmux config path)"
```

## See also

- [Configuration guide](/guide/configuration) for all available options
- [`init`](./init) to generate a project-level `.workmux.yaml`
