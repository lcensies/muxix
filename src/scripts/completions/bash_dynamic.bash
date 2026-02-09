# Dynamic worktree handle completion (directory names)
# Used for open/remove/merge/path/close - these accept handles or branch names
_workmux_handles() {
    workmux _complete-handles 2>/dev/null
}

# Dynamic git branch completion for add command
_workmux_git_branches() {
    workmux _complete-git-branches 2>/dev/null
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
            merge)
                # Handle --into flag (takes worktree handle)
                if [[ "$prev" == "--into" ]]; then
                    COMPREPLY=($(compgen -W "$(_workmux_handles)" -- "$cur"))
                    return
                fi
                # Positional arg: handles
                if [[ "$cur" != -* ]]; then
                    COMPREPLY=($(compgen -W "$(_workmux_handles)" -- "$cur"))
                    return
                fi
                ;;
            open|remove|rm|path|close|send|capture|status|wait)
                # Positional arg: handles
                if [[ "$cur" != -* ]]; then
                    COMPREPLY=($(compgen -W "$(_workmux_handles)" -- "$cur"))
                    return
                fi
                ;;
            add)
                # Handle flags that take specific argument types
                case "$prev" in
                    --base|-b)
                        COMPREPLY=($(compgen -W "$(_workmux_git_branches)" -- "$cur"))
                        return
                        ;;
                    --prompt-file|-P)
                        # File path completion
                        COMPREPLY=($(compgen -f -- "$cur"))
                        return
                        ;;
                esac
                # Positional arg: branches
                if [[ "$cur" != -* ]]; then
                    COMPREPLY=($(compgen -W "$(_workmux_git_branches)" -- "$cur"))
                    return
                fi
                ;;
        esac
    fi

    # Fall back to generated completions
    _workmux "$@"
}

complete -F _workmux_dynamic -o bashdefault -o default workmux
