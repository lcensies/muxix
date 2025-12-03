# Dynamic worktree handle completion (directory names)
# Used for open/remove/merge/path - these accept handles or branch names
_workmux_handles() {
    local handles
    handles=("${(@f)$(workmux __complete-handles 2>/dev/null)}")
    compadd -a handles
}

# Dynamic git branch completion for add command
_workmux_git_branches() {
    local branches
    branches=("${(@f)$(workmux __complete-git-branches 2>/dev/null)}")
    compadd -a branches
}

# Override completion for commands that need dynamic completion
_workmux_dynamic() {
    # Get the subcommand (second word)
    local cmd="${words[2]}"

    # Only handle commands that need dynamic completion
    case "$cmd" in
        open|remove|rm|path|merge)
            # If completing a flag, use generated completions
            if [[ "${words[CURRENT]}" == -* ]]; then
                _workmux "$@"
                return
            fi
            # For positional args after the subcommand, offer worktree handles
            # (commands also accept branch names but handles are the primary identifier)
            if (( CURRENT > 2 )); then
                _workmux_handles
                return
            fi
            ;;
        add)
            # If completing a flag, use generated completions
            if [[ "${words[CURRENT]}" == -* ]]; then
                _workmux "$@"
                return
            fi
            # For positional args after the subcommand, offer git branches
            if (( CURRENT > 2 )); then
                _workmux_git_branches
                return
            fi
            ;;
    esac

    # For all other commands and cases, use generated completions
    _workmux "$@"
}

compdef _workmux_dynamic workmux
