# Dynamic worktree handle completion (directory names)
# Used for open/remove/merge/path/close - repo-scoped lifecycle commands
function __muxix_handles
    muxix _complete-handles 2>/dev/null
end

# Dynamic agent target completion (local handles + cross-project agents)
# Used for send/capture/status/wait/run - agent communication commands
function __muxix_agent_targets
    muxix _complete-agent-targets 2>/dev/null
end

# Dynamic git branch completion for add command
function __muxix_git_branches
    muxix _complete-git-branches 2>/dev/null
end

# Lifecycle commands: local handles only
complete -c muxix -n '__fish_seen_subcommand_from open remove rm rename path merge close' -f -a '(__muxix_handles)'
# Agent commands: local + cross-project targets
complete -c muxix -n '__fish_seen_subcommand_from send capture status wait run' -f -a '(__muxix_agent_targets)'
# Add command: git branches
complete -c muxix -n '__fish_seen_subcommand_from add' -f -a '(__muxix_git_branches)'
