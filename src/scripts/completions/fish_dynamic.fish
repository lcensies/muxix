# Dynamic worktree handle completion (directory names)
# Used for open/remove/merge/path/close - repo-scoped lifecycle commands
function __workmux_handles
    workmux _complete-handles 2>/dev/null
end

# Dynamic agent target completion (local handles + cross-project agents)
# Used for send/capture/status/wait/run - agent communication commands
function __workmux_agent_targets
    workmux _complete-agent-targets 2>/dev/null
end

# Dynamic git branch completion for add command
function __workmux_git_branches
    workmux _complete-git-branches 2>/dev/null
end

# Lifecycle commands: local handles only
complete -c workmux -n '__fish_seen_subcommand_from open remove rm path merge close' -f -a '(__workmux_handles)'
# Agent commands: local + cross-project targets
complete -c workmux -n '__fish_seen_subcommand_from send capture status wait run' -f -a '(__workmux_agent_targets)'
# Add command: git branches
complete -c workmux -n '__fish_seen_subcommand_from add' -f -a '(__workmux_git_branches)'
