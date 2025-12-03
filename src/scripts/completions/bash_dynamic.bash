# Dynamic worktree handle completion (directory names)
# Used for open/remove/merge/path - these accept handles or branch names
_workmux_handles() {
    workmux __complete-handles 2>/dev/null
}

# Dynamic git branch completion for add command
_workmux_git_branches() {
    workmux __complete-git-branches 2>/dev/null
}

# Wrapper that adds dynamic completion
_workmux_dynamic() {
    local cur prev words cword

    # Use _init_completion if available, otherwise fall back to manual parsing
    if declare -F _init_completion >/dev/null 2>&1; then
        _init_completion || return
    else
        COMPREPLY=()
        cur="${COMP_WORDS[COMP_CWORD]}"
        prev="${COMP_WORDS[COMP_CWORD-1]}"
        words=("${COMP_WORDS[@]}")
        cword=$COMP_CWORD
    fi

    # Check if we're completing an argument for specific commands
    if [[ ${cword} -ge 2 ]]; then
        local cmd="${words[1]}"
        case "$cmd" in
            open|remove|rm|path|merge)
                # If not typing a flag, complete with worktree handles
                # (commands also accept branch names but handles are the primary identifier)
                if [[ "$cur" != -* ]]; then
                    COMPREPLY=($(compgen -W "$(_workmux_handles)" -- "$cur"))
                    return
                fi
                ;;
            add)
                # If not typing a flag, complete with git branches
                if [[ "$cur" != -* ]]; then
                    COMPREPLY=($(compgen -W "$(_workmux_git_branches)" -- "$cur"))
                    return
                fi
                ;;
        esac
    fi

    # Fall back to generated completions
    _workmux
}

complete -F _workmux_dynamic -o bashdefault -o default workmux
