---
description: List all git worktrees with their agent, window, and merge status
---

# list

Lists all git worktrees with their agent status, multiplexer window status, and merge status. Alias: `ls`

```bash
workmux list [options] [worktree-or-branch...]
```

## Arguments

| Argument             | Description                                                                                     |
| -------------------- | ----------------------------------------------------------------------------------------------- |
| `worktree-or-branch` | Filter by worktree handle (directory name) or branch name. Multiple values supported. Optional. |

## Options

| Flag     | Description                                                                                                                                                                                                                                          |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--pr`   | Show GitHub PR status for each worktree. Requires the `gh` CLI to be installed and authenticated. Note that it shows pull requests' statuses with [Nerd Font](https://www.nerdfonts.com/) icons, which requires Nerd Font compatible font installed. |
| `--json` | Output as JSON. Produces a JSON array of objects with fields: `handle`, `branch`, `path`, `is_main`, `mode`, `has_uncommitted_changes`, `is_open`.                                                                                                   |

## Examples

```bash
# List all worktrees
workmux list

# List with PR status
workmux list --pr

# Output as JSON for scripting
workmux list --json

# Filter to a specific worktree
workmux list my-feature

# Filter to multiple worktrees
workmux list feature-auth feature-api
```

## Example output

```
BRANCH      AGENT  MUX  UNMERGED  PATH
main        -      -    -         ~/project
user-auth   🤖     ✓    -         ~/project__worktrees/user-auth
bug-fix     ✅     ✓    ●         ~/project__worktrees/bug-fix
api-work    -      ✓    -         ~/project__worktrees/api-work
```

## Key

- AGENT column shows the current agent status using [status icons](/guide/status-tracking):
  - `🤖` = agent is working
  - `💬` = agent is waiting for user input
  - `✅` = agent finished
  - When multiple agents run in one worktree, shows a count (e.g., `2🤖 1✅`)
  - When stdout is piped (e.g., by a script or agent), text labels are used instead: `working`, `waiting`, `done`
- `✓` in MUX column = multiplexer window exists for this worktree
- `●` in UNMERGED column = branch has commits not merged into main
- `-` = not applicable
