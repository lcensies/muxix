# Dynamic worktree handle completion (directory names)
# Used for open/remove/merge/path/close - these accept handles or branch names
_workmux_handles() {
    local handles
    handles=("${(@f)$(workmux _complete-handles 2>/dev/null)}")
    compadd -a handles
}

# Dynamic git branch completion for add command
_workmux_git_branches() {
    local branches
    branches=("${(@f)$(workmux _complete-git-branches 2>/dev/null)}")
    compadd -a branches
}

# Override completion for commands that need dynamic completion
_workmux_dynamic() {
    # Ensure standard zsh array indexing (1-based) regardless of user settings
    emulate -L zsh
    setopt extended_glob  # Required for _files glob qualifiers like *(-/)
    setopt no_nomatch     # Allow failed globs to resolve to empty list

    # Get the subcommand (second word)
    local cmd="${words[2]}"

    # List of flags that take arguments (values), by command.
    # We must defer to _workmux for these so it can offer files/custom hints.
    # Boolean flags are excluded so we can offer positional completions after them.
    local -a arg_flags
    case "$cmd" in
        add)
            arg_flags=(
                -p --prompt
                -P --prompt-file
                --name
                -a --agent
                -n --count
                --foreach
                --branch-template
                --pr
                # Note: --base is excluded because it needs dynamic completion
            )
            ;;
        open)
            arg_flags=(
                -p --prompt
                -P --prompt-file
                # Note: -n/--new is a boolean flag, not included here
            )
            ;;
        merge)
            arg_flags=(
                # Note: --into is excluded because it needs dynamic completion
            )
            ;;
        *)
            arg_flags=()
            ;;
    esac

    # Check if we are currently completing a flag (starts with -)
    # OR if the previous word is a flag that requires an argument.
    if [[ "${words[CURRENT]}" == -* ]] || [[ -n "${arg_flags[(r)${words[CURRENT-1]}]}" ]]; then
        _workmux "$@"
        return
    fi

    # Only handle commands that need dynamic completion
    case "$cmd" in
        open|remove|rm|path|merge|close|send|capture|status|wait)
            # Offer handles mixed with any remaining flags
            _workmux "$@"
            _workmux_handles
            ;;
        add)
            # Offer git branches mixed with any remaining flags
            _workmux "$@"
            _workmux_git_branches
            ;;
        *)
            # For all other commands, strictly use generated completions
            _workmux "$@"
            ;;
    esac
}

compdef _workmux_dynamic workmux
