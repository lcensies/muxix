# Dynamic worktree handle completion (directory names)
# Used for open/remove/merge/path/close - repo-scoped lifecycle commands
_muxix_handles() {
    local -a handles
    handles=("${(@f)$(muxix _complete-handles 2>/dev/null)}")
    # "${(@f)...}" on empty output produces a single empty string; filter it out
    handles=(${handles:#})
    (( ${#handles} )) && compadd -a handles
}

# Dynamic agent target completion (local handles + cross-project agents)
# Used for send/capture/status/wait/run - agent communication commands
_muxix_agent_targets() {
    local -a targets
    targets=("${(@f)$(muxix _complete-agent-targets 2>/dev/null)}")
    targets=(${targets:#})
    (( ${#targets} )) && compadd -a targets
}

# Dynamic git branch completion for add command
_muxix_git_branches() {
    local -a branches
    branches=("${(@f)$(muxix _complete-git-branches 2>/dev/null)}")
    branches=(${branches:#})
    (( ${#branches} )) && compadd -a branches
}

# Main completion function.
#
# This replaces the clap-generated _muxix, wrapping _muxix_base with
# dynamic completions for positional arguments (handles, branches).
# Flag/option completion is delegated to _muxix_base which uses _arguments.
#
# Works with both autoloading (fpath) and eval:
# - Autoloaded: the file body defines all functions, then redefines _muxix
#   as this wrapper and calls it. Subsequent calls go directly to the wrapper.
# - Eval'd: all functions are defined at global scope, _muxix is registered
#   via compdef.
_muxix() {
    # Ensure standard zsh array indexing (1-based) regardless of user settings
    emulate -L zsh
    setopt extended_glob  # Required for _files glob qualifiers like *(-/)
    setopt no_nomatch     # Allow failed globs to resolve to empty list

    # Get the subcommand (second word)
    local cmd="${words[2]}"

    # List of flags that take arguments (values), by command.
    # When completing a flag value, we defer to _muxix_base so it can offer
    # file paths, custom hints, etc. via _arguments.
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

    # If completing a flag (starts with -) or a flag's argument value,
    # use _muxix_base which has the full _arguments definitions.
    if [[ "${words[CURRENT]}" == -* ]] || [[ -n "${arg_flags[(r)${words[CURRENT-1]}]}" ]]; then
        _muxix_base "$@"
        return
    fi

    # For commands that take handles or branches, offer only those
    # (no file fallback from _default). Flag completion is handled above.
    case "$cmd" in
        open|remove|rm|rename|path|merge|close)
            _muxix_handles
            ;;
        send|capture|status|wait|run)
            _muxix_agent_targets
            ;;
        add)
            _muxix_git_branches
            ;;
        *)
            # For all other commands (config, sandbox, etc.), use base completions
            _muxix_base "$@"
            ;;
    esac
}

# Autoload / eval detection:
# - When autoloaded via fpath, funcstack[1] is the outer autoloaded function
#   that just defined _muxix (replacing itself). Call _muxix to handle
#   the current completion request.
# - When eval'd (e.g. eval "$(muxix completions zsh)"), we are at the top
#   level so funcstack[1] is not _muxix. Register the function with compdef.
if [ "$funcstack[1]" = "_muxix" ]; then
    _muxix "$@"
else
    compdef _muxix muxix
fi
