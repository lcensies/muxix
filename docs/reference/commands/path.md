---
description: Print the filesystem path of a worktree
---

# path

Prints the filesystem path of an existing worktree. Useful for scripting or quickly navigating to a worktree directory.

```bash
muxix path <name>
```

## Arguments

- `<name>`: Worktree name (the directory name).

## Examples

```bash
# Get the path of a worktree
muxix path user-auth
# Output: /Users/you/project__worktrees/user-auth

# Use in scripts or with cd
cd "$(muxix path user-auth)"

# Copy a file to a worktree
cp config.json "$(muxix path feature-branch)/"
```
