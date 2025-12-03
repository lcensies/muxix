# Dynamic worktree handle completion (directory names)
# Used for open/remove/merge/path - these accept handles or branch names
function __workmux_handles
    workmux __complete-handles 2>/dev/null
end

# Dynamic git branch completion for add command
function __workmux_git_branches
    workmux __complete-git-branches 2>/dev/null
end

# Add dynamic completions for commands that take worktree handles or branch names
# (handles are the primary identifier shown in completions)
complete -c workmux -n '__fish_seen_subcommand_from open remove rm path merge' -f -a '(__workmux_handles)'
# Add dynamic completions for add command (uses git branches)
complete -c workmux -n '__fish_seen_subcommand_from add' -f -a '(__workmux_git_branches)'
