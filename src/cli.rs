use crate::command::args::{MultiArgs, PromptArgs, RescueArgs, SetupFlags};
use crate::config::MuxMode;
use crate::{claude, command, config, git, nerdfont, projects};
use anyhow::{Context, Result};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Shell, generate};
use std::path::PathBuf;

#[derive(Clone, Debug)]
struct WorktreeBranchParser;

impl WorktreeBranchParser {
    fn new() -> Self {
        Self
    }

    fn get_branches(&self) -> Vec<String> {
        // Don't attempt completions if not in a git repo.
        if !git::is_git_repo().unwrap_or(false) {
            return Vec::new();
        }

        let worktrees = match git::list_worktrees() {
            Ok(wt) => wt,
            // Fail silently on completion; don't disrupt the user's shell.
            Err(_) => return Vec::new(),
        };

        let main_branch = git::get_default_branch().ok();

        worktrees
            .into_iter()
            .map(|(_, branch)| branch)
            // Filter out the main branch, as it's not a candidate for merging/removing.
            .filter(|branch| main_branch.as_deref() != Some(branch.as_str()))
            // Filter out detached HEAD states.
            .filter(|branch| branch != "(detached)")
            .collect()
    }
}

impl clap::builder::TypedValueParser for WorktreeBranchParser {
    type Value = String;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        // Use the default string parser for validation.
        clap::builder::StringValueParser::new().parse_ref(cmd, None, value)
    }

    fn possible_values(
        &self,
    ) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
        // Return None to avoid running git operations during completion script generation.
        // Dynamic completions are handled by the _complete-branches subcommand,
        // which is called by the shell only when the user presses TAB.
        None
    }
}

/// Parser for worktree handles (directory names), used for open/path/remove commands.
#[derive(Clone, Debug)]
struct WorktreeHandleParser;

impl WorktreeHandleParser {
    fn new() -> Self {
        Self
    }

    fn get_handles() -> Vec<String> {
        // Don't attempt completions if not in a git repo.
        if !git::is_git_repo().unwrap_or(false) {
            return Vec::new();
        }

        let worktrees = match git::list_worktrees() {
            Ok(wt) => wt,
            // Fail silently on completion; don't disrupt the user's shell.
            Err(_) => return Vec::new(),
        };

        let main_worktree_root = git::get_main_worktree_root().ok();

        worktrees
            .into_iter()
            .filter_map(|(path, _)| {
                // Filter out the main worktree
                if main_worktree_root.as_ref() == Some(&path) {
                    return None;
                }
                // Extract directory name as the handle
                path.file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .collect()
    }
}

impl clap::builder::TypedValueParser for WorktreeHandleParser {
    type Value = String;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        // Use the default string parser for validation.
        clap::builder::StringValueParser::new().parse_ref(cmd, None, value)
    }

    fn possible_values(
        &self,
    ) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
        // Return None to avoid running git operations during completion script generation.
        // Dynamic completions are handled by the _complete-handles subcommand,
        // which is called by the shell only when the user presses TAB.
        None
    }
}

/// Parser for agent targets, used for send/capture/status/wait/run commands.
///
/// Includes local worktree handles plus active agents from other projects.
/// Cross-project agents appear as `project:handle` when their name would be ambiguous.
#[derive(Clone, Debug)]
struct AgentTargetParser;

impl AgentTargetParser {
    fn new() -> Self {
        Self
    }

    fn get_targets() -> Vec<String> {
        // Start with local worktree handles
        let mut targets = WorktreeHandleParser::get_handles();

        // Also include the main worktree handle (agents can run there too)
        if git::is_git_repo().unwrap_or(false)
            && let Ok(main_root) = git::get_main_worktree_root()
            && let Some(name) = main_root.file_name()
        {
            let handle = name.to_string_lossy().to_string();
            if !targets.contains(&handle) {
                targets.push(handle);
            }
        }

        // Append global agent handles from reconciled state
        let mux = crate::multiplexer::create_backend(crate::multiplexer::detect_backend());
        if let Ok(store) = crate::state::StateStore::new()
            && let Ok(agents) = store.load_reconciled_agents(mux.as_ref())
        {
            for agent in &agents {
                let root = crate::workflow::find_worktree_root(&agent.path)
                    .unwrap_or_else(|| agent.path.clone());
                if let Some(name) = root.file_name() {
                    let handle = name.to_string_lossy().to_string();
                    if !targets.contains(&handle) {
                        targets.push(handle.clone());
                    }
                    // Also add qualified project:handle for disambiguation
                    if let Some(parent) = root.parent()
                        && let Some(proj) = parent.file_name()
                    {
                        let qualified = format!("{}:{}", proj.to_string_lossy(), handle);
                        if !targets.contains(&qualified) {
                            targets.push(qualified);
                        }
                    }
                }
            }
        }

        targets.sort();
        targets.dedup();
        targets
    }
}

impl clap::builder::TypedValueParser for AgentTargetParser {
    type Value = String;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        clap::builder::StringValueParser::new().parse_ref(cmd, None, value)
    }

    fn possible_values(
        &self,
    ) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
        // Dynamic completions handled by _complete-agent-targets subcommand
        None
    }
}

#[derive(Clone, Debug)]
struct GitBranchParser;

impl GitBranchParser {
    fn new() -> Self {
        Self
    }

    fn get_branches() -> Vec<String> {
        // Don't attempt completions if not in a git repo.
        if !git::is_git_repo().unwrap_or(false) {
            return Vec::new();
        }

        // Fail silently on completion; don't disrupt the user's shell.
        git::list_checkout_branches().unwrap_or_default()
    }
}

impl clap::builder::TypedValueParser for GitBranchParser {
    type Value = String;

    fn parse_ref(
        &self,
        cmd: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        // Use the default string parser for validation.
        clap::builder::StringValueParser::new().parse_ref(cmd, None, value)
    }

    fn possible_values(
        &self,
    ) -> Option<Box<dyn Iterator<Item = clap::builder::PossibleValue> + '_>> {
        // Return None to avoid running git operations during completion script generation.
        // Dynamic completions are handled by the _complete-git-branches subcommand,
        // which is called by the shell only when the user presses TAB.
        None
    }
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(name = "muxix")]
#[command(about = "An opinionated workflow tool that orchestrates git worktrees and tmux")]
#[command(help_template = "\
{about}

{usage-heading} {usage}

Worktree lifecycle:
  add          Create a new worktree and tmux window
  remove       Remove a worktree, tmux window, and branch without merging [rm]
  rename       Rename a worktree, tmux window/session, and optionally branch
  merge        Merge a branch, then clean up the worktree and tmux window
  open         Open a tmux window for an existing worktree
  close        Close a worktree's tmux window (keeps the worktree and branch)
  resurrect    Restore worktree windows after a tmux or computer crash

Projects:
  project      Track project directories for 'muxix start'
  start        Start all tracked projects (session per project, window per worktree)

Monitoring:
  dashboard    Show a TUI dashboard of all active muxix agents
  sidebar      Toggle a live agent status sidebar in tmux
  list         List all worktrees [ls]
  path         Get the filesystem path of a worktree
  status       Query agent status for worktrees

Setup and configuration:
  init         Generate example .muxix.yaml configuration file
  setup        Set up agent status tracking hooks and install skills
  config       Manage global configuration
  sandbox      Manage sandbox settings
  sync-files   Re-apply file operations (copy/symlink) to worktrees
  claude       Claude Code integration commands

Agent interaction:
  send         Send a prompt or instruction to a running agent
  capture      Capture terminal output from a running agent
  wait         Wait for agents to reach a target status
  run          Run a command in a worktree's window
  exec         Launch a coding agent under a named agent-profile

Help and updates:
  docs         Show detailed documentation (renders README.md)
  changelog    Show the changelog (what's new in each version)
  update       Update muxix to the latest version
  completions  Generate shell completions
  help         Print help for a command

Options:
  -h, --help     Print help
  -V, --version  Print version
      --profile <NAME>  Config profile(s) to apply (comma-separated)

Run 'muxix docs' for detailed documentation.
")]
struct Cli {
    /// Config profile(s) to apply, comma-separated for several (applied left
    /// to right). Overrides MUXIX_PROFILE and `default_profile:` in config;
    /// pass an empty value to disable a configured default_profile.
    #[arg(long, global = true, value_name = "NAME")]
    profile: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum CliMuxMode {
    Window,
    Session,
}

impl From<CliMuxMode> for MuxMode {
    fn from(value: CliMuxMode) -> Self {
        match value {
            CliMuxMode::Window => MuxMode::Window,
            CliMuxMode::Session => MuxMode::Session,
        }
    }
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Create a new worktree and tmux window
    Add {
        /// Name of the branch (creates if it doesn't exist) or remote ref (e.g., origin/feature).
        /// When used with --pr, this becomes the custom local branch name.
        #[arg(required_unless_present_any = ["pr", "auto_name"], value_parser = GitBranchParser::new())]
        branch_name: Option<String>,

        /// Agent runtime to create the agent in (default: the project's
        /// `agent_runtime`, else `local`). A non-local runtime owns the agent
        /// itself: no worktree, no window, no pane.
        #[arg(long)]
        runtime: Option<String>,

        /// Pull request number to checkout
        #[arg(long, conflicts_with_all = ["base", "auto_name"])]
        pr: Option<u32>,

        /// Generate branch name from prompt using LLM
        #[arg(short = 'A', long = "auto-name", conflicts_with = "pr")]
        auto_name: bool,

        /// Base branch/commit/tag to branch from (overrides config base_branch, defaults to current branch)
        #[arg(long)]
        base: Option<String>,

        /// Explicit name for the worktree directory and tmux window (overrides worktree_naming strategy and worktree_prefix)
        #[arg(long)]
        name: Option<String>,

        /// Explicit name for the muxix-managed tmux target
        #[arg(long = "target-name")]
        target_name: Option<String>,

        /// Parent tmux session for window-mode targets
        #[arg(long = "parent-session")]
        parent_session: Option<String>,

        #[command(flatten)]
        prompt: PromptArgs,

        #[command(flatten)]
        setup: SetupFlags,

        #[command(flatten)]
        rescue: RescueArgs,

        #[command(flatten)]
        multi: MultiArgs,

        /// Use a named pane layout from config instead of default panes
        #[arg(short = 'l', long, conflicts_with = "agent")]
        layout: Option<String>,

        /// Fork the last conversation from the current worktree into the new one.
        /// Specify a session ID to fork a specific conversation, or omit for most recent.
        #[arg(long, num_args = 0..=1, default_missing_value = "", require_equals = true)]
        fork: Option<String>,

        /// Block until the created tmux window is closed
        #[arg(short = 'W', long)]
        wait: bool,

        /// Override the multiplexer mode for this command only
        #[arg(long, value_enum)]
        mode: Option<CliMuxMode>,

        /// Create the window in its own tmux session (useful for session-per-project workflows)
        #[arg(short = 's', long, conflicts_with = "mode")]
        session: bool,

        /// Use an alternate config file for this invocation (still merges with global config)
        #[arg(long, value_hint = clap::ValueHint::FilePath)]
        config: Option<PathBuf>,
    },

    /// Open a tmux window for an existing worktree
    Open {
        /// Worktree name(s) (directory name, visible in tmux window). Optional with --new.
        #[arg(value_parser = WorktreeHandleParser::new(), required_unless_present = "new")]
        names: Vec<String>,

        /// Re-run post-create hooks (e.g., pnpm install)
        #[arg(long)]
        run_hooks: bool,

        /// Re-apply file operations (copy/symlink)
        #[arg(long)]
        force_files: bool,

        /// Force opening in a new window (creates suffix like -2, -3) instead of switching to existing
        #[arg(long, short = 'n')]
        new: bool,

        /// Override the multiplexer mode for this command only
        #[arg(long, value_enum)]
        mode: Option<CliMuxMode>,

        /// Open in session mode (overrides stored mode for this worktree)
        #[arg(short = 's', long, conflicts_with = "mode")]
        session: bool,

        /// Explicit name for the muxix-managed tmux target
        #[arg(long = "target-name")]
        target_name: Option<String>,

        /// Parent tmux session for window-mode targets
        #[arg(long = "parent-session")]
        parent_session: Option<String>,

        /// Resume the agent's most recent conversation in this worktree
        #[arg(short = 'c', long = "continue")]
        continue_session: bool,

        #[command(flatten)]
        prompt: PromptArgs,

        /// Use an alternate config file for this invocation (still merges with global config)
        #[arg(long, value_hint = clap::ValueHint::FilePath)]
        config: Option<PathBuf>,
    },

    /// Close a worktree's tmux window (keeps the worktree and branch)
    Close {
        /// Worktree name (defaults to current directory if omitted)
        #[arg(value_parser = WorktreeHandleParser::new())]
        name: Option<String>,
    },

    /// Restore worktree windows after a tmux or computer crash
    ///
    /// Uses persisted agent state files to detect which worktrees had active
    /// agents before the crash, then relaunches each one.
    ///
    /// Each agent's own session store decides how it comes back: if the
    /// worktree has a resumable conversation the agent is relaunched with its
    /// continue flag; if not (commonly because the configured agent changed,
    /// leaving the old agent's sessions unreachable) the stored task prompt is
    /// re-sent instead, naming the agent that spawned the worktree. Restored
    /// windows are verified to still be alive before being reported.
    Resurrect {
        /// Show what would be restored without doing it
        #[arg(long)]
        dry_run: bool,
    },

    /// List every agent, from every runtime that owns one
    Agents {
        /// Omit to list; `stop` to end an agent whatever runtime owns it
        #[command(subcommand)]
        command: Option<AgentsCommand>,
        #[arg(long)]
        json: bool,
    },

    /// List agent runtimes (where agents run) with health and features
    Runtimes {
        #[arg(long)]
        json: bool,
    },

    /// Manage tracked project directories (used by `muxix start`)
    Project {
        #[command(subcommand)]
        command: ProjectCommands,
    },

    /// Start all tracked projects: session per project, window per worktree
    Start {
        /// Relaunch the last coding agent in each project/worktree, resuming
        /// its previous conversation where possible
        #[arg(short = 'c', long = "continue")]
        continue_session: bool,
    },

    /// Merge a branch, then clean up the worktree and tmux window
    Merge {
        /// Worktree name or branch (defaults to current directory)
        #[arg(value_parser = WorktreeHandleParser::new())]
        name: Option<String>,

        /// The target branch to merge into (defaults to main_branch from config)
        #[arg(long, value_parser = GitBranchParser::new())]
        into: Option<String>,

        /// Ignore uncommitted and staged changes
        #[arg(long)]
        ignore_uncommitted: bool,

        /// Rebase the branch onto the main branch before merging (fast-forward)
        #[arg(long, group = "merge_strategy")]
        rebase: bool,

        /// Squash all commits from the branch into a single commit on the main branch
        #[arg(long, group = "merge_strategy")]
        squash: bool,

        /// Keep the worktree, window, and branch after merging (skip cleanup)
        #[arg(short = 'k', long)]
        keep: bool,

        /// Clean up the worktree, window, and branch after merging
        #[arg(long, conflicts_with = "keep")]
        cleanup: bool,

        /// Skip running pre-merge hooks
        #[arg(short = 'n', long)]
        no_verify: bool,

        /// Skip running all hooks (pre-merge and pre-remove)
        #[arg(long)]
        no_hooks: bool,

        /// Show a system notification on successful merge
        #[arg(long)]
        notification: bool,
    },

    /// Rename a worktree, its tmux window/session, and (optionally) its branch
    Rename {
        /// [OLD_NAME] NEW_NAME. If only one argument is given, renames the current worktree.
        #[arg(required = true, num_args = 1..=2, value_parser = WorktreeHandleParser::new())]
        names: Vec<String>,

        /// Also rename the underlying git branch to match the new handle
        #[arg(short = 'b', long)]
        branch: bool,
    },


    /// Remove a worktree, tmux window, and branch without merging
    #[command(visible_alias = "rm")]
    Remove {
        /// Worktree names (defaults to current directory name if empty)
        #[arg(value_parser = WorktreeHandleParser::new(), conflicts_with_all = ["gone", "all"], num_args = 0..)]
        names: Vec<String>,

        /// Remove worktrees whose upstream remote branch has been deleted (e.g., after PR merge)
        #[arg(long, conflicts_with = "all")]
        gone: bool,

        /// Remove all worktrees (except the main worktree)
        #[arg(long)]
        all: bool,

        /// Skip confirmation and ignore uncommitted changes
        #[arg(short, long)]
        force: bool,

        /// Keep the local branch (only remove worktree and tmux window)
        #[arg(short = 'k', long)]
        keep_branch: bool,
    },

    /// List all worktrees
    #[command(visible_alias = "ls")]
    List {
        /// Show PR status for each worktree (requires gh CLI)
        #[arg(long)]
        pr: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Filter by worktree name or branch (supports multiple)
        #[arg(value_parser = WorktreeBranchParser::new())]
        filter: Vec<String>,
    },

    /// Get the filesystem path of a worktree
    Path {
        /// Worktree name (directory name)
        #[arg(value_parser = WorktreeHandleParser::new())]
        name: String,
    },

    /// Send a prompt or instruction to a running agent
    Send {
        /// Worktree name (supports cross-project with project:handle syntax)
        #[arg(value_parser = AgentTargetParser::new())]
        name: String,

        /// Text to send (reads from --file or stdin if omitted)
        #[arg(conflicts_with = "file")]
        text: Option<String>,

        /// Read prompt from file
        #[arg(short, long, conflicts_with = "text")]
        file: Option<String>,
    },

    /// Capture terminal output from a running agent
    Capture {
        /// Worktree name (supports cross-project with project:handle syntax)
        #[arg(value_parser = AgentTargetParser::new())]
        name: String,

        /// Number of lines to capture
        #[arg(short = 'n', long, default_value = "200")]
        lines: u16,
    },

    /// Query agent status for worktrees
    Status {
        /// Worktree names (supports cross-project with project:handle syntax)
        #[arg(value_parser = AgentTargetParser::new())]
        worktrees: Vec<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Include git info (staged/unstaged changes, unmerged commits)
        #[arg(long)]
        git: bool,
    },

    /// Wait for agents to reach a target status
    Wait {
        /// Worktree names (supports cross-project with project:handle syntax)
        #[arg(required = true, value_parser = AgentTargetParser::new())]
        worktrees: Vec<String>,

        /// Target status to wait for: comma-separated list of
        /// working, waiting, done, completed, failed, merged
        #[arg(long, default_value = "done")]
        status: String,

        /// Maximum wait time in seconds
        #[arg(long)]
        timeout: Option<u64>,

        /// Return when ANY worktree reaches target (default: wait for ALL)
        #[arg(long)]
        any: bool,
    },

    /// Run a command in a worktree's window
    Run {
        /// Worktree name (supports cross-project with project:handle syntax)
        #[arg(value_parser = AgentTargetParser::new())]
        name: String,

        /// Command to run (everything after --)
        #[arg(last = true, required = true)]
        command: Vec<String>,

        /// Run in background without waiting (default: wait and stream output)
        #[arg(short = 'b', long)]
        background: bool,

        /// Keep run artifacts after completion (for debugging)
        #[arg(long)]
        keep: bool,

        /// Maximum wait time in seconds
        #[arg(long)]
        timeout: Option<u64>,
    },

    /// Re-apply file operations (copy/symlink) to worktrees
    #[command(name = "sync-files")]
    SyncFiles {
        /// Sync all worktrees instead of just the current one
        #[arg(long)]
        all: bool,
    },

    /// Generate example .muxix.yaml configuration file
    Init,

    /// Set up agent status tracking hooks and install skills
    Setup {
        /// Only set up status tracking hooks (alias for --only hooks)
        #[arg(long)]
        hooks: bool,
        /// Only install skills (alias for --only skills)
        #[arg(long)]
        skills: bool,
        /// Apply every configured section without prompting. Required when
        /// stdin is not a terminal (activation scripts, CI, devcontainers).
        #[arg(long = "non-interactive", short = 'y')]
        non_interactive: bool,
        /// Report what would change and exit 2 if the machine has drifted.
        /// Writes nothing.
        #[arg(long, conflicts_with = "non_interactive")]
        check: bool,
        /// Emit one JSON object on stdout; progress goes to stderr
        #[arg(long)]
        json: bool,
        /// Restrict to these sections, comma-separated:
        /// hooks, skills, subagents, plugins, agent-settings, prompts, theme, mcp, agent-profiles, deps
        #[arg(long, value_name = "SECTIONS")]
        only: Option<String>,
        /// Keep harness features muxix installed that the config no longer
        /// declares, instead of removing them.
        #[arg(long = "no-prune")]
        no_prune: bool,
    },

    /// Sync local config with the org provision server and apply policy
    Provision {
        #[command(subcommand)]
        command: Option<ProvisionCommand>,
        /// Show what would change without applying
        #[arg(long)]
        dry_run: bool,
        /// Exit with error on any policy violation (default: warn only)
        #[arg(long)]
        strict: bool,
    },

    /// Export, import, or diff muxix config profiles for team sharing
    Profile {
        #[command(subcommand)]
        command: ProfileCommand,
    },

    /// Show detailed documentation (renders README.md)
    Docs,

    /// Show the changelog (what's new in each version)
    Changelog,

    /// Update muxix to the latest version
    Update,

    /// Toggle a live agent status sidebar in tmux
    Sidebar {
        /// Scope sidebar to this session, or toggle this session off when global sidebar is active
        #[arg(short = 's', long)]
        session: bool,
        /// Force global scope regardless of sidebar.default_scope config
        #[arg(short = 'g', long, conflicts_with = "session")]
        global: bool,
        #[command(subcommand)]
        action: Option<SidebarAction>,
    },

    /// Run the sidebar TUI (internal use)
    #[command(hide = true, name = "_sidebar-run")]
    SidebarRun,

    /// Sync sidebar into a window (internal use, called by tmux hooks)
    #[command(hide = true, name = "_sidebar-sync")]
    SidebarSync {
        /// Target window ID (from tmux hook context)
        #[arg(long)]
        window: Option<String>,
    },

    /// Reflow sidebar layout after window resize (internal use, called by tmux hooks)
    #[command(hide = true, name = "_sidebar-reflow")]
    SidebarReflow {
        /// Target window ID
        #[arg(long)]
        window: Option<String>,
    },

    /// Reflow sidebar layouts in all windows (internal use, called by tmux hooks)
    #[command(hide = true, name = "_sidebar-reflow-all")]
    SidebarReflowAll,

    /// Run the sidebar daemon (internal use)
    #[command(hide = true, name = "_sidebar-daemon")]
    SidebarDaemon,

    /// Show a TUI dashboard of all active muxix agents across all sessions
    Dashboard {
        /// Preview pane size as percentage (10-90). Larger = more preview, less table.
        #[arg(long, short = 'P', value_parser = clap::value_parser!(u8).range(10..=90))]
        preview_size: Option<u8>,

        /// Open diff view directly for the current worktree
        #[arg(long, short = 'd')]
        diff: bool,

        /// Filter to only show agents in the current session
        #[arg(short = 's', long)]
        session: bool,

        /// Open directly on the specified tab
        #[arg(long, short = 't', value_enum)]
        tab: Option<command::dashboard::DashboardTab>,
    },

    /// Manage global configuration
    Config(command::config::ConfigArgs),

    /// Declare what agent harnesses contain (plugins, skills, subagents, prompts)
    Bootstrap(command::bootstrap::BootstrapArgs),

    /// Claude Code integration commands
    Claude {
        #[command(subcommand)]
        command: ClaudeCommands,
    },

    /// Manage sandbox settings
    Sandbox(command::sandbox::SandboxArgs),

    /// Read/write the per-project runtime state store (for preflight & harness gates)
    #[command(name = "project-state")]
    ProjectState {
        #[command(subcommand)]
        command: command::project_state::ProjectStateCommand,
    },

    /// Manage the project's MCP servers (sync `.mcp.json`, show status)
    Mcp {
        #[command(subcommand)]
        command: command::mcp::McpCommand,
    },

    /// Manage named agent definitions and the capability registry
    #[command(name = "agent-registry")]
    AgentRegistry {
        #[command(subcommand)]
        command: command::agent_registry::AgentRegistryCommand,
    },

    /// Set agent status for the current tmux window (used by hooks)
    #[command(hide = true)]
    SetWindowStatus {
        #[arg(value_enum)]
        command: command::set_window_status::SetWindowStatusCommand,
    },

    /// Set the base branch for the current worktree (used after rebasing)
    #[command(hide = true, name = "set-base")]
    SetBase {
        /// The new base branch
        #[arg(value_parser = GitBranchParser::new())]
        base: String,
    },

    /// Execute a run spec (internal use)
    #[command(hide = true, name = "_exec")]
    Exec {
        /// Absolute path to run directory
        #[arg(long)]
        run_dir: std::path::PathBuf,
    },

    /// Launch a coding agent under a named agent-profile
    ///
    /// Overlays skills/extensions/config from `agent_profiles.<name>` on top of
    /// the agent's real config dir, then execs the agent. Select the profile
    /// with the global `--profile <name>` flag; with no profile it falls
    /// through to the untouched base config dir. Build profiles with
    /// `muxix setup`.
    #[command(name = "exec")]
    AgentExec {
        /// The agent command to launch (e.g. `pi`, `claude`)
        agent: String,

        /// Arguments passed through to the agent
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Switch multiplexer focus to an agent by agent ID or window-name fragment
    Focus {
        /// Agent ID / UUID prefix (see `muxix status`) or window-name substring
        /// (e.g. feature-auth)
        target: String,
    },

    /// Switch to the agent that most recently completed or is waiting for input
    #[command(hide = true, name = "last-done")]
    LastDone,

    /// Switch to the last visited agent (toggle between two)
    #[command(hide = true, name = "last-agent")]
    LastAgent,

    /// Execute a command on the host (used by guest shims)
    #[command(hide = true, name = "host-exec")]
    HostExec {
        /// Command name and arguments
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        args: Vec<String>,
    },

    /// Read clipboard from host (used by sandbox clipboard shims)
    #[command(hide = true, name = "clipboard-read")]
    ClipboardRead {
        /// MIME type to read
        mime: String,
    },

    /// Generate shell completions
    Completions {
        /// The shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Output worktree branch names for shell completion (internal use)
    #[command(hide = true, name = "_complete-branches")]
    CompleteBranches,

    /// Output worktree handles for shell completion (internal use)
    #[command(hide = true, name = "_complete-handles")]
    CompleteHandles,

    /// Output git branches for shell completion (internal use)
    #[command(hide = true, name = "_complete-git-branches")]
    CompleteGitBranches,

    /// Output agent targets for shell completion (internal use)
    ///
    /// Includes local worktree handles plus active agents from other projects.
    #[command(hide = true, name = "_complete-agent-targets")]
    CompleteAgentTargets,

    /// Background update check (internal use)
    #[command(hide = true, name = "_check-update")]
    CheckUpdate,



    /// Read and mutate the task graph from the shell (list/get/create/update/delete).
    ///
    /// muxix stores the graph and serialises writes to it; deciding what to run
    /// next is the calling harness's job.
    ///
    /// Relative `--graph` paths resolve against the main worktree root, so agents
    /// in a feature worktree share the project's single graph.
    Task {
        #[command(subcommand)]
        action: TaskAction,
    },

    /// Emit an out-of-band agent signal (called from agent hooks, agent tools, or
    /// the `/implement` slash command).
    ///
    /// Supports two modes:
    /// 1. PANE-KEYED TURN SIGNALS (keyed by $TMUX_PANE):
    ///    - turn-done   — agent finished a response turn (Stop hook)
    ///    - needs-input — agent blocked on a user question (Notification hook)
    ///    - working     — agent resumed; clears needs-input (PostToolUse/UserPromptSubmit)
    ///    - proceed     — the gate this pane waits on is approved (e.g. /implement)
    ///    - reject      — the gate is refused, with optional feedback
    ///
    /// 2. NODE-KEYED SIGNALS (keyed by --node, for stage-to-stage messaging):
    ///    - done        — agent finished this stage explicitly
    ///    - error       — agent hit an error; asks for a retry, with feedback
    ///
    /// `done`/`error` are dual-mode: with --node, writes the node-keyed signal file
    /// a harness polls; without --node, writes an agent-authored completion onto the
    /// pane's AgentState (keyed by $TMUX_PANE or --pane), read by `wait`/`status`.
    ///
    /// `activity` is pane-keyed and sets the free-text label the sidebar renders
    /// via its `{activity}` token: `--label "tf:review 2/5"`, `--clear` to clear.
    Signal {
        /// Signal kind: turn-done | needs-input | working | proceed | reject | done | error | activity
        kind: String,
        /// Override the pane id (defaults to $TMUX_PANE). Used for pane-keyed signals
        /// (including done/error without --node).
        #[arg(long)]
        pane: Option<String>,
        /// Node id for agent-level signals (done/error). When set, writes to node-keyed
        /// signal file; when omitted, done/error write a pane-keyed completion instead.
        #[arg(long)]
        node: Option<String>,
        /// Feedback message (used by reject, error, or done with feedback).
        #[arg(long)]
        feedback: Option<String>,
        /// Activity label text (kind `activity`). Empty clears it.
        #[arg(long)]
        label: Option<String>,
        /// Clear the activity label (kind `activity`).
        #[arg(long)]
        clear: bool,
    },

    /// Internal: emit an `agent.session` capability report (which muxix hooks
    /// are installed) to the event log. Run from the agent's SessionStart hook so
    /// a missing/stale hook setup is visible from the start of every session.
    #[command(hide = true)]
    HooksReport {
        /// Override the pane id (defaults to $TMUX_PANE).
        #[arg(long)]
        pane: Option<String>,
    },






}

#[derive(Subcommand, Debug)]
pub enum ProjectCommands {
    /// Track a project directory
    Add {
        /// Path to the project directory
        dir: PathBuf,
    },
    /// Untrack a project by name or path
    Rm {
        /// Project name or path
        target: String,
    },
    /// List tracked projects
    List,
    /// Open a tracked project's session and focus it
    Open {
        /// Project name or path
        target: String,
        /// Relaunch the last coding agent, resuming its previous conversation
        #[arg(short = 'c', long = "continue")]
        continue_session: bool,
    },
    /// Sync the project registry with configured ADEs
    Sync {
        /// Only sync this ADE
        #[arg(long)]
        ade: Option<String>,
        /// Show what would change without touching either side
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum AgentsCommand {
    /// Stop an agent by reference (`<runtime>:<id>`, or a pane id for local)
    Stop {
        /// Agent reference as printed by `muxix agents`
        reference: String,
    },
}




#[derive(Subcommand, Debug)]
pub enum SidebarAction {
    /// Switch to the next agent in sidebar order
    Next,
    /// Switch to the previous agent in sidebar order
    Prev,
    /// Jump to the Nth agent in sidebar order (1-indexed)
    Jump {
        /// Agent number (1-9)
        #[arg(value_name = "N", value_parser = clap::value_parser!(u64).range(1..))]
        index: u64,
    },
}

#[derive(Subcommand, Debug)]
enum ClaudeCommands {
    /// Remove stale entries from ~/.claude.json for deleted worktrees
    Prune,
}

#[derive(Subcommand, Debug)]
pub enum ProvisionCommand {
    /// Show cached org policy status
    Status,
    /// Fetch latest policy from the provision server (requires server_url + token)
    Sync,
}

#[derive(Subcommand, Debug)]
pub enum ProfileCommand {
    /// Print the current config profile snapshot
    Show,
    /// Export profile snapshot to a file
    Export {
        #[arg(short = 'o', long)]
        output: Option<std::path::PathBuf>,
    },
    /// Diff local profile against the team profile from org policy
    Diff,
}

/// Check if the command should show the nerdfont setup prompt.
/// Only commands that display icons should trigger the prompt.
fn should_prompt_nerdfont(cmd: &Commands) -> bool {
    matches!(
        cmd,
        Commands::Add { .. } | Commands::Init | Commands::Dashboard { .. } | Commands::List { .. }
    )
}

/// Check if the command should show the status tracking setup wizard.
/// Excludes `Setup` to avoid double-prompting (the setup command handles its own flow).
/// Excludes `Dashboard` because the wizard prompt interferes with the TUI.
fn should_prompt_status_setup(cmd: &Commands) -> bool {
    matches!(
        cmd,
        Commands::Add { .. } | Commands::Init | Commands::List { .. }
    )
}

/// Check if the command should trigger a background update check.
fn should_check_update(cmd: &Commands) -> bool {
    matches!(cmd, Commands::Add { .. })
}

// --- Public Entry Point ---
pub fn run() -> Result<()> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(mut e) => {
            // Filter hidden (underscore-prefixed) commands from "similar subcommands" suggestions.
            // Workaround for https://github.com/clap-rs/clap/issues/4853
            if e.kind() == ErrorKind::InvalidSubcommand {
                let visible = e
                    .get(ContextKind::SuggestedSubcommand)
                    .and_then(|v| match v {
                        ContextValue::Strings(suggestions) => {
                            let filtered: Vec<String> = suggestions
                                .iter()
                                .filter(|s| !s.starts_with('_'))
                                .cloned()
                                .collect();
                            Some(filtered)
                        }
                        _ => None,
                    });
                if let Some(visible) = visible {
                    e.insert(
                        ContextKind::SuggestedSubcommand,
                        ContextValue::Strings(visible),
                    );
                }
            }
            e.exit()
        }
    };

    // Record --profile before anything loads config: the side-effect loads
    // below (nerdfont, update check) must see the same profile the command will.
    crate::config::profiles::set_cli_profile(cli.profile.clone());
    let cli_profile = cli.profile.clone();

    let command = cli.command.unwrap_or(Commands::Dashboard {
        preview_size: None,
        diff: false,
        session: false,
        tab: None,
    });

    // Extract config override early so the side-effect loads (nerdfont, update
    // check) respect the user's explicit --config choice.
    let config_override = match &command {
        Commands::Add { config, .. } => config.as_deref(),
        Commands::Open { config, .. } => config.as_deref(),
        _ => None,
    };

    // Always initialize nerdfont setting for prefix consistency across commands.
    // Only prompt interactively for commands that display icons.
    // If config fails to load, skip the nerdfont wizard -- it will be shown on
    // the next successful run and the real error surfaces when the command loads
    // config with `?`.
    let (cfg, config_ok) = match config::Config::load_with_override(None, config_override) {
        Ok(cfg) => {
            // Trace the actual resolved config so the log alone explains
            // config-driven behaviour. Full data at debug to keep info lean.
            crate::wm_evt!("config.load", ok = true, config_override = ?config_override);
            crate::wm_evt_dbg!("config.data", cfg = ?cfg);
            (cfg, true)
        }
        Err(e) => {
            // Early side-effect load failed; the real error re-surfaces when the
            // command loads config with `?`, but record it here too.
            crate::wm_evt!("config.load", ok = false, err = %e, config_override = ?config_override);
            (config::Config::default(), false)
        }
    };
    let has_pua = nerdfont::config_has_pua(&cfg);
    let nerdfont_enabled = if cfg.nerdfont.is_some() || has_pua {
        // Already configured or PUA detected
        cfg.nerdfont.unwrap_or(has_pua)
    } else if config_ok && should_prompt_nerdfont(&command) {
        // Prompt user (returns None in non-interactive mode)
        nerdfont::check_and_prompt(&cfg)?.unwrap_or(false)
    } else {
        false
    };
    nerdfont::init(Some(nerdfont_enabled), has_pua);

    // Check agent status tracking setup after nerdfont.
    // Uses a separate gate to avoid double-prompting when running `muxix setup`.
    if config_ok
        && should_prompt_status_setup(&command)
        && let Err(e) = crate::agent::setup::prompt_wizard()
    {
        tracing::debug!(?e, "status setup wizard failed");
    }

    // Background update check: reads local cache, optionally shows a notice,
    // and spawns a background process to refresh the cache if stale.
    if should_check_update(&command) {
        command::update::check_and_notify(&cfg);
    }

    // The single dispatch chokepoint: `?command` Debug-renders the variant name
    // and its parsed args, so every invocation's intent is in the log.
    crate::wm_evt!("cmd.dispatch", cmd = ?command);

    match command {
        Commands::Add {
            branch_name,
            runtime,
            pr,
            auto_name,
            base,
            name,
            target_name,
            parent_session,
            prompt,
            setup,
            rescue,
            multi,
            layout,
            fork,
            wait,
            mode,
            session,
            config,
        } => {
            // Sugar: `muxix add <existing-repo-dir>` tracks the project
            // instead of creating a worktree (directory containing .git wins
            // over a same-named branch).
            if let Some(bn) = branch_name.as_deref() {
                let path = std::path::Path::new(bn);
                if path.is_dir() && path.join(".git").exists() {
                    println!(
                        "'{}' is a git repository — tracking it as a project \
                         (run `muxix add <branch>` inside a repo to create worktrees)",
                        bn
                    );
                    return projects::cli_add(path);
                }
            }
            // A non-local runtime owns the agent process; there is no
            // worktree to make or pane to open, so this diverts before any of
            // the local machinery runs.
            if let Some(reference) = command::add::start_on_runtime(
                runtime.as_deref(),
                branch_name.as_deref(),
                name.as_deref(),
                prompt.prompt.as_deref(),
            )? {
                println!("{reference}");
                return Ok(());
            }
            let mode_override = mode
                .map(MuxMode::from)
                .or(session.then_some(MuxMode::Session));
            command::add::run(
                branch_name.as_deref(),
                pr,
                auto_name,
                base.as_deref(),
                name,
                target_name,
                parent_session,
                prompt,
                setup,
                rescue,
                multi,
                layout,
                fork,
                wait,
                mode_override,
                config.as_deref(),
            )
        }
        Commands::Open {
            names,
            run_hooks,
            force_files,
            new,
            mode,
            session,
            target_name,
            parent_session,
            continue_session,
            prompt,
            config,
        } => {
            let mode_override = mode
                .map(MuxMode::from)
                .or(session.then_some(MuxMode::Session));
            command::open::run(
                &names,
                run_hooks,
                force_files,
                new,
                mode_override,
                target_name,
                parent_session,
                continue_session,
                prompt,
                config.as_deref(),
            )
        }
        Commands::Close { name } => command::close::run(name.as_deref()),
        Commands::Resurrect { dry_run } => command::resurrect::run(dry_run),
        Commands::Agents { command, json } => match command {
            None => command::agents::run(json),
            Some(AgentsCommand::Stop { reference }) => command::agents::stop(&reference),
        },
        Commands::Runtimes { json } => crate::agent::runtime::registry::print_runtimes(json),
        Commands::Project { command } => match command {
            ProjectCommands::Add { dir } => projects::cli_add(&dir),
            ProjectCommands::Rm { target } => projects::cli_rm(&target),
            ProjectCommands::List => projects::cli_list(),
            ProjectCommands::Open {
                target,
                continue_session,
            } => projects::start::open(&target, continue_session),
            ProjectCommands::Sync { ade, dry_run } => {
                projects::sync::cli_sync(ade.as_deref(), dry_run)
            }
        },
        Commands::Start { continue_session } => projects::start::run(continue_session),
        Commands::Merge {
            name,
            into,
            ignore_uncommitted,
            rebase,
            squash,
            keep,
            cleanup,
            no_verify,
            no_hooks,
            notification,
        } => command::merge::run(
            name.as_deref(),
            into.as_deref(),
            ignore_uncommitted,
            rebase,
            squash,
            keep,
            cleanup,
            no_verify,
            no_hooks,
            notification,
        ),
        Commands::Remove {
            names,
            gone,
            all,
            force,
            keep_branch,
        } => command::remove::run(names, gone, all, force, keep_branch),
        Commands::Rename { names, branch } => command::rename::run(names, branch),
        Commands::List { pr, json, filter } => command::list::run(pr, json, &filter),
        Commands::Path { name } => command::path::run(&name),
        Commands::Send { name, text, file } => {
            command::send::run(&name, text.as_deref(), file.as_deref())
        }
        Commands::Capture { name, lines } => command::capture::run(&name, lines),
        Commands::Status {
            worktrees,
            json,
            git,
        } => command::status::run(&worktrees, json, git),
        Commands::Wait {
            worktrees,
            status,
            timeout,
            any,
        } => command::wait::run(&worktrees, &status, timeout, any),
        Commands::Run {
            name,
            command,
            background,
            keep,
            timeout,
        } => command::run::run(&name, command, background, keep, timeout),
        Commands::Exec { run_dir } => command::exec::run(&run_dir),
        Commands::AgentExec { agent, args } => {
            command::agent_exec::run(&agent, &args, cli_profile.as_deref())
        }
        Commands::SyncFiles { all } => command::sync_files::run(all),
        Commands::Init => crate::config::Config::init(),
        Commands::Setup {
            hooks,
            skills,
            non_interactive,
            check,
            json,
            only,
            no_prune,
        } => {
            // --hooks / --skills predate --only and remain accepted as aliases.
            let mut sections = match only.as_deref() {
                Some(raw) => command::setup::parse_sections(raw)?,
                None => Vec::new(),
            };
            if hooks {
                sections.push(command::setup::Section::Hooks);
            }
            if skills {
                sections.push(command::setup::Section::Skills);
            }

            let opts = command::setup::SetupOptions {
                non_interactive,
                check,
                json,
                only: sections,
                no_prune,
                profile: cli_profile.clone(),
            };
            if opts.is_automated() {
                let code = command::setup::run_automated(&opts)?;
                if code != 0 {
                    std::process::exit(code);
                }
                Ok(())
            } else {
                command::setup::run(&opts)
            }
        }
        Commands::Provision {
            command,
            dry_run,
            strict,
        } => match command {
            Some(ProvisionCommand::Status) => command::provision::run_status(),
            Some(ProvisionCommand::Sync) => command::provision::run_sync(dry_run, strict),
            None if dry_run => command::provision::run_sync(true, strict),
            None => command::provision::run_status(),
        },
        Commands::Profile { command } => match command {
            ProfileCommand::Show => command::profile::run_show(),
            ProfileCommand::Export { output } => {
                command::profile::run_export(output.as_deref())
            }
            ProfileCommand::Diff => command::profile::run_diff(),
        },
        Commands::Docs => command::docs::run(),
        Commands::Changelog => command::changelog::run(),
        Commands::Update => command::update::run(),
        Commands::Sidebar {
            session,
            global,
            action,
        } => match action {
            Some(SidebarAction::Next) => {
                command::sidebar::navigate(command::sidebar::NavAction::Next)
            }
            Some(SidebarAction::Prev) => {
                command::sidebar::navigate(command::sidebar::NavAction::Prev)
            }
            Some(SidebarAction::Jump { index }) => {
                command::sidebar::navigate(command::sidebar::NavAction::Jump(index as usize))
            }
            None => {
                if session {
                    command::sidebar::toggle_session()
                } else if global {
                    command::sidebar::toggle()
                } else {
                    // No flags: check config default_scope
                    let use_session = crate::config::Config::load(None)
                        .ok()
                        .and_then(|c| c.sidebar.default_scope)
                        .is_some_and(|s| {
                            s == crate::config::SidebarDefaultScope::Session
                        });
                    if use_session {
                        command::sidebar::toggle_session()
                    } else {
                        command::sidebar::toggle()
                    }
                }
            }
        },
        Commands::SidebarRun => command::sidebar::run_sidebar(),
        Commands::SidebarSync { window } => command::sidebar::sync(window.as_deref()),
        Commands::SidebarReflow { window } => command::sidebar::reflow(window.as_deref()),
        Commands::SidebarReflowAll => command::sidebar::reflow_all(),
        Commands::SidebarDaemon => command::sidebar::run_daemon(),
        Commands::Dashboard {
            preview_size,
            diff,
            session,
            tab,
        } => command::dashboard::run(preview_size, diff, session, tab),
        Commands::Config(args) => command::config::run(args),
        Commands::Bootstrap(args) => command::bootstrap::run(args),
        Commands::Claude { command } => match command {
            ClaudeCommands::Prune => prune_claude_config(),
        },
        Commands::Sandbox(args) => command::sandbox::run(args),
        Commands::ProjectState { command } => command::project_state::run(command),
        Commands::Mcp { command } => command::mcp::run(command),
        Commands::AgentRegistry { command } => command::agent_registry::run(command),
        Commands::SetWindowStatus { command } => command::set_window_status::run(command),
        Commands::SetBase { base } => command::set_base::run(&base),
        Commands::Focus { target } => command::focus::run(&target),
        Commands::LastDone => command::last_done::run(),
        Commands::LastAgent => command::last_agent::run(),
        Commands::HostExec { args } => {
            let (command, cmd_args) = args
                .split_first()
                .ok_or_else(|| anyhow::anyhow!("host-exec requires a command name"))?;
            let code = command::host_exec::run(command, cmd_args)?;
            std::process::exit(code);
        }
        Commands::ClipboardRead { mime } => {
            let code = command::clipboard_read::run(&mime)?;
            std::process::exit(code);
        }
        Commands::Completions { shell } => {
            generate_completions(shell);
            Ok(())
        }
        Commands::CompleteBranches => {
            for branch in WorktreeBranchParser::new().get_branches() {
                println!("{branch}");
            }
            Ok(())
        }
        Commands::CompleteHandles => {
            for handle in WorktreeHandleParser::get_handles() {
                println!("{handle}");
            }
            Ok(())
        }
        Commands::CompleteGitBranches => {
            for branch in GitBranchParser::get_branches() {
                println!("{branch}");
            }
            Ok(())
        }
        Commands::CompleteAgentTargets => {
            for target in AgentTargetParser::get_targets() {
                println!("{target}");
            }
            Ok(())
        }
        Commands::CheckUpdate => command::update::run_background_check(),
        Commands::Task { action } => match action {
            TaskAction::List {
                status,
                label,
                frontier,
                json,
                graph,
            } => command::task_cli::list(&graph, status, label, frontier, json),
            TaskAction::Get { query, json, graph } => command::task_cli::get(&graph, &query, json),
            TaskAction::Resolve {
                id,
                retry,
                abandon,
                graph,
            } => command::task_cli::resolve(&graph, &id, retry, abandon),
            TaskAction::Claim {
                id,
                branch,
                base,
                worktree,
                json,
                graph,
            } => command::task_cli::claim(&graph, &id, &branch, &base, &worktree, json),
            TaskAction::Create {
                id,
                title,
                description,
                depends_on,
                status,
                worktree,
                labels,
                priority,
                parent,
                inherit_from,
                graph,
            } => command::task_cli::create(
                &graph,
                id,
                title,
                description,
                depends_on,
                status,
                worktree,
                labels,
                priority,
                parent,
                inherit_from,
            ),
            TaskAction::Update {
                id,
                title,
                description,
                status,
                worktree,
                depends_on,
                labels,
                priority,
                hint,
                blocked_reason,
                blocked_reason_file,
                add_labels,
                remove_labels,
                graph,
            } => command::task_cli::update(
                &graph,
                &id,
                title,
                description,
                status,
                worktree,
                depends_on,
                labels,
                priority,
                hint,
                blocked_reason,
                blocked_reason_file,
                add_labels,
                remove_labels,
            ),
            TaskAction::Delete { id, graph } => command::task_cli::delete(&graph, &id),
        },
        Commands::Signal {
            kind,
            pane,
            node,
            feedback,
            label,
            clear,
        } => command::signal::run(
            &kind,
            pane.as_deref(),
            node.as_deref(),
            feedback.as_deref(),
            label.as_deref(),
            clear,
        ),
        Commands::HooksReport { pane } => command::hooks_report::run(pane.as_deref()),
    }
}

fn prune_claude_config() -> Result<()> {
    claude::prune_stale_entries().context("Failed to prune Claude configuration")?;
    Ok(())
}

fn generate_completions(shell: Shell) {
    let mut cmd = Cli::command();
    let name = cmd.get_name().to_string();

    // Generate base completions
    let mut buf = Vec::new();
    generate(shell, &mut cmd, &name, &mut buf);
    let base_script = String::from_utf8_lossy(&buf);

    // Append dynamic branch completion for each shell
    // Note: PowerShell and Elvish are not supported because clap_complete generates
    // anonymous completers that can't be wrapped without breaking standard completions.
    match shell {
        Shell::Zsh => {
            let base = prepare_zsh_base(&base_script, &name);
            println!("{base}");
            print_zsh_dynamic_completion();
        }
        _ => {
            print!("{base_script}");
            match shell {
                Shell::Bash => print_bash_dynamic_completion(),
                Shell::Fish => print_fish_dynamic_completion(),
                _ => {}
            }
        }
    }
}

/// Rename the clap-generated zsh completion function so the dynamic wrapper
/// (in `zsh_dynamic.zsh`) can take the primary `_muxix` name.
///
/// The dynamic wrapper needs to BE `_muxix` — the function zsh autoloads
/// from fpath. It delegates flag completion to `_muxix_base` (the renamed
/// clap output) and handles positional args with dynamic helpers.
///
/// The `replace("_{name}", "_{name}_base")` is precise: `_{name}` (with the
/// leading underscore) only appears as zsh function identifiers in clap's
/// output. Bare `{name}` in `#compdef`, `_describe` strings, `curcontext`,
/// and state names is unaffected.
fn prepare_zsh_base(script: &str, name: &str) -> String {
    let fn_prefix = format!("_{name}");
    let base_fn_prefix = format!("_{name}_base");

    let script = script.replace(&fn_prefix, &base_fn_prefix);

    // Strip the autoload/eval detection block clap appends at the end.
    // After renaming it registers _muxix_base, which conflicts with our
    // dynamic wrapper's own registration.
    let funcstack_block = format!(
        "\nif [ \"$funcstack[1]\" = \"{base_fn_prefix}\" ]; then\n    \
         {base_fn_prefix} \"$@\"\nelse\n    \
         compdef {base_fn_prefix} {name}\nfi\n"
    );
    match script.strip_suffix(&funcstack_block) {
        Some(stripped) => stripped.to_string(),
        None => script,
    }
}

fn print_zsh_dynamic_completion() {
    print!("{}", include_str!("scripts/completions/zsh_dynamic.zsh"));
}

fn print_bash_dynamic_completion() {
    print!("{}", include_str!("scripts/completions/bash_dynamic.bash"));
}

fn print_fish_dynamic_completion() {
    print!("{}", include_str!("scripts/completions/fish_dynamic.fish"));
}

#[derive(Subcommand, Debug)]
pub enum TaskAction {
    /// List tasks (optionally filter by status/label, or show only the ready frontier)
    List {
        /// Only tasks with this status (todo/in_progress/merging/done/failed)
        #[arg(long)]
        status: Option<String>,
        /// Only tasks carrying this label
        #[arg(long)]
        label: Option<String>,
        /// Only the ready frontier (todo tasks whose deps are all done).
        /// `--ready` is the same filter, named for coordinator agents.
        #[arg(long, visible_alias = "ready")]
        frontier: bool,
        /// Emit JSON instead of a table
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = "tasks/index.json")]
        graph: PathBuf,
    },
    /// Get one task by exact id, or fuzzy-search id+title when no id matches
    Get {
        /// Task id (exact) or fuzzy query
        query: String,
        /// Emit JSON instead of a field listing
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = "tasks/index.json")]
        graph: PathBuf,
    },
    /// Create a new task (agents use this to schedule discovered work)
    Create {
        /// Task id; defaults to a slug of --title when omitted
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        title: String,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long, value_delimiter = ',')]
        depends_on: Vec<String>,
        #[arg(long, default_value = "todo")]
        status: String,
        /// Worktree handle; defaults to the task id
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long = "label", value_delimiter = ',')]
        labels: Vec<String>,
        #[arg(long)]
        priority: Option<i64>,
        /// Containment edge: the task this one was decomposed FROM
        #[arg(long)]
        parent: Option<String>,
        /// Copy the harness/testing fields of this task onto the new one
        #[arg(long)]
        inherit_from: Option<String>,
        #[arg(long, default_value = "tasks/index.json")]
        graph: PathBuf,
    },
    /// Decide what happens to a task whose slot is `unknown`.
    ///
    /// The daemon parks rather than guessing when it cannot prove a worker is
    /// alive. This is the explicit decision that unparks it.
    Resolve {
        /// Id of the task to resolve
        id: String,
        /// Put it back in the ready queue. The slot is cleared, so the next
        /// dispatch binds afresh and bumps the attempt — which fences the previous
        /// worker out if it is somehow still alive.
        #[arg(long, conflicts_with = "abandon")]
        retry: bool,
        /// Give up on it. Marked failed, worktree and branch left for inspection.
        #[arg(long, conflicts_with = "retry")]
        abandon: bool,
        #[arg(long, default_value = "tasks/index.json")]
        graph: PathBuf,
    },
    /// Bind a task to the worktree and branch you are about to create.
    ///
    /// Call this BEFORE `git worktree add`, so a crash in between leaves a record
    /// naming the tree that may exist. Repeating the same claim is safe; claiming a
    /// task already bound to something else fails.
    Claim {
        /// Id of the task to bind
        id: String,
        /// Branch the work will live on
        #[arg(long)]
        branch: String,
        /// Ref the branch is cut from (base branch, or the parent's branch)
        #[arg(long)]
        base: String,
        /// Path of the worktree you are about to create
        #[arg(long)]
        worktree: PathBuf,
        /// Emit JSON instead of a human line
        #[arg(long)]
        json: bool,
        #[arg(long, default_value = "tasks/index.json")]
        graph: PathBuf,
    },
    /// Update fields of an existing task (only the flags you pass are changed)
    Update {
        /// Id of the task to update
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long)]
        worktree: Option<String>,
        #[arg(long, value_delimiter = ',')]
        depends_on: Option<Vec<String>>,
        #[arg(long = "label", value_delimiter = ',')]
        labels: Option<Vec<String>>,
        #[arg(long)]
        priority: Option<i64>,
        /// Append a line to the task's agent_hints (retry guidance)
        #[arg(long)]
        hint: Option<String>,
        /// Why the task stopped (read by the failure steward)
        #[arg(long)]
        blocked_reason: Option<String>,
        /// Read the blocked reason from a file (e.g. .muxix/blocked/<id>.md)
        #[arg(long, conflicts_with = "blocked_reason")]
        blocked_reason_file: Option<PathBuf>,
        /// Add a label, keeping the existing ones
        #[arg(long = "add-label", value_delimiter = ',')]
        add_labels: Vec<String>,
        /// Remove a label, keeping the others
        #[arg(long = "remove-label", value_delimiter = ',')]
        remove_labels: Vec<String>,
        #[arg(long, default_value = "tasks/index.json")]
        graph: PathBuf,
    },
    /// Delete a task by id
    Delete {
        /// Id of the task to delete
        id: String,
        #[arg(long, default_value = "tasks/index.json")]
        graph: PathBuf,
    },
}

#[cfg(test)]
mod tests {
    use super::*;



    #[test]
    fn prepare_zsh_base_renames_function_identifiers() {
        let input = concat!(
            "#compdef muxix\n",
            "_muxix() {\n",
            "  \":: :_muxix_commands\"\n",
            "}\n",
            "(( $+functions[_muxix_commands] )) ||\n",
            "_muxix_commands() {\n",
            "  _describe -t commands 'muxix commands' commands\n",
            "}\n",
            "\nif [ \"$funcstack[1]\" = \"_muxix\" ]; then\n",
            "    _muxix \"$@\"\n",
            "else\n",
            "    compdef _muxix muxix\n",
            "fi\n",
        );
        let result = prepare_zsh_base(input, "muxix");

        // Function identifiers are renamed
        assert!(result.contains("_muxix_base()"));
        assert!(result.contains("_muxix_base_commands"));
        assert!(!result.contains("_muxix()"));

        // Bare "muxix" in #compdef and _describe strings is preserved
        assert!(result.contains("#compdef muxix"));
        assert!(result.contains("'muxix commands'"));

        // funcstack block is stripped
        assert!(!result.contains("funcstack"));
        assert!(!result.contains("compdef _muxix_base"));
    }

    #[test]
    fn prepare_zsh_base_preserves_state_and_curcontext() {
        let input = concat!(
            "#compdef muxix\n",
            "_muxix() {\n",
            "  \"*::: :->muxix\"\n",
            "  curcontext=\"muxix-command-$line[1]:\"\n",
            "}\n",
            "\nif [ \"$funcstack[1]\" = \"_muxix\" ]; then\n",
            "    _muxix \"$@\"\n",
            "else\n",
            "    compdef _muxix muxix\n",
            "fi\n",
        );
        let result = prepare_zsh_base(input, "muxix");

        // State names and curcontext use bare "muxix" (no underscore), unchanged
        assert!(result.contains("->muxix"));
        assert!(result.contains("muxix-command-"));
    }

    #[test]
    fn prepare_zsh_base_tolerates_missing_funcstack_block() {
        let input = "_muxix() {\n  echo hello\n}\n";
        let result = prepare_zsh_base(input, "muxix");

        assert!(result.contains("_muxix_base()"));
        assert!(!result.contains("_muxix()"));
    }

    #[test]
    fn prepare_zsh_base_works_with_real_clap_output() {
        let mut cmd = Cli::command();
        let name = cmd.get_name().to_string();
        let mut buf = Vec::new();
        generate(Shell::Zsh, &mut cmd, &name, &mut buf);
        let base_script = String::from_utf8_lossy(&buf);

        let result = prepare_zsh_base(&base_script, &name);

        // Main function is renamed
        assert!(result.contains("_muxix_base()"));
        assert!(!result.contains("\n_muxix()"));

        // Helpers are renamed
        assert!(result.contains("_muxix_base_commands"));

        // #compdef header preserved
        assert!(result.starts_with("#compdef muxix\n"));

        // _describe strings preserved
        assert!(result.contains("'muxix commands'"));

        // funcstack block stripped
        assert!(!result.contains("funcstack"));
    }

    /// Helper: produce the full `muxix completions zsh` output
    /// (base post-processed by prepare_zsh_base + dynamic wrapper).
    fn generate_full_zsh_completions() -> String {
        let mut cmd = Cli::command();
        let name = cmd.get_name().to_string();
        let mut buf = Vec::new();
        generate(Shell::Zsh, &mut cmd, &name, &mut buf);
        let base_script = String::from_utf8_lossy(&buf);
        let base = prepare_zsh_base(&base_script, &name);
        let dynamic = include_str!("scripts/completions/zsh_dynamic.zsh");
        format!("{base}\n{dynamic}")
    }

    #[test]
    fn zsh_full_output_has_no_stale_muxix_functions() {
        let output = generate_full_zsh_completions();

        // The only _muxix() definition should be the dynamic wrapper.
        // There must be no clap-generated _muxix() left (it was renamed).
        let muxix_fn_count = output.matches("\n_muxix()").count();
        assert_eq!(
            muxix_fn_count, 1,
            "Expected exactly one _muxix() definition (the dynamic wrapper)"
        );

        // The wrapper must call _muxix_base, not itself
        let wrapper_section: &str = output
            .split("\n_muxix()")
            .nth(1)
            .expect("_muxix() not found");
        assert!(
            wrapper_section.contains("_muxix_base"),
            "Dynamic wrapper must delegate to _muxix_base"
        );
    }

    #[test]
    fn zsh_full_output_no_file_fallback_for_handle_commands() {
        let output = generate_full_zsh_completions();

        // The dynamic wrapper's case branches for handle commands should
        // call _muxix_handles, not _muxix_base (which has _default).
        // Extract the case block from the wrapper.
        let wrapper_start = output.find("\n_muxix()").expect("_muxix() not found");
        let wrapper = &output[wrapper_start..];

        // The handle commands should appear in the case pattern
        for cmd in ["open", "remove", "close", "merge"] {
            assert!(wrapper.contains(cmd), "Wrapper should handle {cmd}");
        }
        assert!(
            wrapper.contains("_muxix_handles"),
            "Wrapper should call _muxix_handles for handle commands"
        );
        assert!(
            wrapper.contains("_muxix_git_branches"),
            "Wrapper should call _muxix_git_branches for add"
        );
    }

    #[test]
    fn zsh_full_output_autoload_and_eval_compatible() {
        let output = generate_full_zsh_completions();

        // Must start with #compdef for fpath autoloading
        assert!(
            output.starts_with("#compdef muxix\n"),
            "Must start with #compdef for fpath autoloading"
        );

        // Must have funcstack detection for autoload/eval compatibility
        assert!(
            output.contains(r#""$funcstack[1]" = "_muxix""#),
            "Must have funcstack check for autoload detection"
        );

        // Must have compdef registration for eval case
        assert!(
            output.contains("compdef _muxix muxix"),
            "Must register _muxix via compdef for eval case"
        );

        // Must NOT have compdef for _muxix_base (that was stripped)
        assert!(
            !output.contains("compdef _muxix_base"),
            "Must not register _muxix_base directly"
        );
    }

    #[test]
    fn bash_output_unaffected_by_zsh_postprocessing() {
        let mut cmd = Cli::command();
        let name = cmd.get_name().to_string();
        let mut buf = Vec::new();
        generate(Shell::Bash, &mut cmd, &name, &mut buf);
        let base_script = String::from_utf8_lossy(&buf);
        let dynamic = include_str!("scripts/completions/bash_dynamic.bash");
        let output = format!("{base_script}{dynamic}");

        // Bash uses _muxix directly (no _base rename)
        assert!(output.contains("_muxix()"));
        assert!(!output.contains("_muxix_base"));

        // Dynamic wrapper registered
        assert!(output.contains("complete -F _muxix_dynamic"));
    }

    #[test]
    fn fish_output_unaffected_by_zsh_postprocessing() {
        let mut cmd = Cli::command();
        let name = cmd.get_name().to_string();
        let mut buf = Vec::new();
        generate(Shell::Fish, &mut cmd, &name, &mut buf);
        let base_script = String::from_utf8_lossy(&buf);
        let dynamic = include_str!("scripts/completions/fish_dynamic.fish");
        let output = format!("{base_script}{dynamic}");

        // Fish uses muxix directly (no _base rename)
        assert!(output.contains("__fish_muxix"));
        assert!(!output.contains("muxix_base"));

        // Dynamic completions registered
        assert!(output.contains("__muxix_handles"));
        assert!(output.contains("__muxix_git_branches"));
    }
}
