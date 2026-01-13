---
description: List all git worktrees with their tmux window and merge status
---

# list

Lists all git worktrees with their tmux window status and merge status. Alias: `ls`

```bash
workmux list [flags]
```

## Options

| Flag   | Description                                                                                                                                                                                                                                          |
| ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--pr` | Show GitHub PR status for each worktree. Requires the `gh` CLI to be installed and authenticated. Note that it shows pull requests' statuses with [Nerd Font](https://www.nerdfonts.com/) icons, which requires Nerd Font compatible font installed. |

## Examples

```bash
# List all worktrees
workmux list

# List with PR status
workmux list --pr
```

## Example output

```
BRANCH      TMUX    UNMERGED    PATH
------      ----    --------    ----
main        -       -           ~/project
user-auth   ✓       -           ~/project__worktrees/user-auth
bug-fix     ✓       ●           ~/project__worktrees/bug-fix
```

## Key

- `✓` in TMUX column = tmux window exists for this worktree
- `●` in UNMERGED column = branch has commits not merged into main
- `-` = not applicable
