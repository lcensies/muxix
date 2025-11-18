use crate::workflow::SetupOptions;
use crate::{claude, config, git, workflow};
use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use tera::{Context as TeraContext, Tera, Value, from_value, to_value};

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
        let branches = self.get_branches();
        // Note: Box::leak is used here because clap's PossibleValue::new requires 'static str.
        // This is unavoidable with the current clap API for dynamic completions.
        // The memory leak is small (proportional to number of branches) and only occurs
        // during shell completion queries, which are infrequent.
        let branches_static: Vec<&'static str> = branches
            .into_iter()
            .map(|s| Box::leak(s.into_boxed_str()) as &'static str)
            .collect();

        Some(Box::new(
            branches_static
                .into_iter()
                .map(clap::builder::PossibleValue::new),
        ))
    }
}

#[derive(Debug, Clone)]
pub enum Prompt {
    Inline(String),
    FromFile(PathBuf),
}

#[derive(Debug, Clone)]
struct WorktreeSpec {
    branch_name: String,
    agent: Option<String>,
    template_context: TeraContext,
}

const BRANCH_TEMPLATE_NAME: &str = "branch_template";

#[derive(clap::Args, Debug)]
struct RemoveArgs {
    /// Name of the branch to remove (defaults to current branch)
    #[arg(value_parser = WorktreeBranchParser::new())]
    branch_name: Option<String>,

    /// Skip confirmation and ignore uncommitted changes
    #[arg(short, long)]
    force: bool,

    /// Also delete the remote branch
    #[arg(short = 'r', long)]
    delete_remote: bool,

    /// Keep the local branch (only remove worktree and tmux window)
    #[arg(short = 'k', long, conflicts_with = "delete_remote")]
    keep_branch: bool,
}

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
#[command(name = "workmux")]
#[command(about = "An opinionated workflow tool that orchestrates git worktrees and tmux")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new worktree and tmux window
    Add {
        /// Name of the branch (creates if it doesn't exist) or remote ref (e.g., origin/feature)
        branch_name: String,

        /// Base branch/commit/tag to branch from (defaults to current branch)
        #[arg(long)]
        base: Option<String>,

        /// Inline prompt text to store in the new worktree
        #[arg(short = 'p', long, conflicts_with_all = ["prompt_file", "prompt_editor"])]
        prompt: Option<String>,

        /// Path to a file whose contents should be used as the prompt
        #[arg(short = 'P', long = "prompt-file", conflicts_with_all = ["prompt", "prompt_editor"])]
        prompt_file: Option<PathBuf>,

        /// Open $EDITOR to write the prompt
        #[arg(short = 'e', long = "prompt-editor", conflicts_with_all = ["prompt", "prompt_file"])]
        prompt_editor: bool,

        /// Skip running post-create hooks
        #[arg(short = 'H', long)]
        no_hooks: bool,

        /// Skip file copy/symlink operations
        #[arg(short = 'F', long)]
        no_file_ops: bool,

        /// Skip executing pane commands (panes open with plain shells)
        #[arg(short = 'C', long)]
        no_pane_cmds: bool,

        /// Create tmux window in the background (do not switch to it)
        #[arg(short = 'b', long = "background")]
        background: bool,

        /// Move uncommitted changes from the current worktree to the new worktree
        #[arg(short = 'w', long, conflicts_with_all = ["count", "foreach"])]
        with_changes: bool,

        /// Interactively select which changes to move (only applies with --with-changes)
        #[arg(long, requires = "with_changes")]
        patch: bool,

        /// Also move untracked files (only applies with --with-changes)
        #[arg(short = 'u', long, requires = "with_changes")]
        include_untracked: bool,

        /// The agent(s) to use. Creates one worktree per agent if -n is not specified.
        #[arg(short = 'a', long)]
        agent: Vec<String>,

        /// Number of worktree instances to create.
        /// Can be used with zero or one --agent. Incompatible with --foreach.
        #[arg(
            short = 'n',
            long,
            value_parser = clap::value_parser!(u32).range(1..),
            conflicts_with = "foreach"
        )]
        count: Option<u32>,

        /// Generate multiple worktrees from a variable matrix.
        /// Format: "var1:valA,valB;var2:valX,valY". Lists must have equal length.
        /// Incompatible with --agent and --count.
        #[arg(long, conflicts_with_all = ["agent", "count"])]
        foreach: Option<String>,

        /// Template for branch names in multi-worktree modes.
        /// Variables: {{ base_name }}, {{ agent }}, {{ num }}, {{ foreach_vars }}.
        #[arg(
            long,
            default_value = r#"{{ base_name }}{% if agent %}-{{ agent | slugify }}{% endif %}{% for key, value in foreach_vars %}-{{ value | slugify }}{% endfor %}{% if num %}-{{ num }}{% endif %}"#
        )]
        branch_template: String,
    },

    /// Open a tmux window for an existing worktree
    Open {
        /// Name of the branch with an existing worktree
        #[arg(value_parser = WorktreeBranchParser::new())]
        branch_name: String,

        /// Re-run post-create hooks (e.g., pnpm install)
        #[arg(long)]
        run_hooks: bool,

        /// Re-apply file operations (copy/symlink)
        #[arg(long)]
        force_files: bool,
    },

    /// Merge a branch, then clean up the worktree and tmux window
    Merge {
        /// Name of the branch to merge (defaults to current branch)
        #[arg(value_parser = WorktreeBranchParser::new())]
        branch_name: Option<String>,

        /// Ignore uncommitted and staged changes
        #[arg(long)]
        ignore_uncommitted: bool,

        /// Also delete the remote branch
        #[arg(short = 'r', long)]
        delete_remote: bool,

        /// Rebase the branch onto the main branch before merging (fast-forward)
        #[arg(long, group = "merge_strategy")]
        rebase: bool,

        /// Squash all commits from the branch into a single commit on the main branch
        #[arg(long, group = "merge_strategy")]
        squash: bool,
    },

    /// Remove a worktree, tmux window, and branch without merging
    #[command(visible_alias = "rm")]
    Remove(RemoveArgs),

    /// List all worktrees
    #[command(visible_alias = "ls")]
    List,

    /// Generate example .workmux.yaml configuration file
    Init,

    /// Claude Code integration commands
    Claude {
        #[command(subcommand)]
        command: ClaudeCommands,
    },

    /// Generate shell completions
    Completions {
        /// The shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand)]
enum ClaudeCommands {
    /// Remove stale entries from ~/.claude.json for deleted worktrees
    Prune,
}

// --- Public Entry Point ---
pub fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add {
            branch_name,
            base,
            prompt,
            prompt_file,
            prompt_editor,
            no_hooks,
            no_file_ops,
            no_pane_cmds,
            background,
            with_changes,
            patch,
            include_untracked,
            agent,
            count,
            foreach,
            branch_template,
        } => {
            // If --with-changes is set, use the create_with_changes workflow
            if with_changes {
                let mut options = SetupOptions::new(!no_hooks, !no_file_ops, !no_pane_cmds);
                options.focus_window = !background;
                let config = config::Config::load(None)?;
                let result = workflow::create_with_changes(
                    &branch_name,
                    include_untracked,
                    patch,
                    &config,
                    options,
                )
                .context("Failed to move uncommitted changes")?;

                println!(
                    "✓ Moved uncommitted changes to new worktree for branch '{}'\n  Worktree: {}\n  Original worktree is now clean",
                    result.branch_name,
                    result.worktree_path.display()
                );
                return Ok(());
            }

            // Construct setup options from flags
            let mut options = SetupOptions::new(!no_hooks, !no_file_ops, !no_pane_cmds);
            options.focus_window = !background;

            let prompt_template = if prompt_editor {
                let editor_content =
                    edit::edit("").context("Failed to open editor or read content")?;
                let trimmed = editor_content.trim();
                if trimmed.is_empty() {
                    return Err(anyhow!("Aborting: prompt is empty"));
                }
                Some(Prompt::Inline(trimmed.to_string()))
            } else {
                match (prompt, prompt_file) {
                    (Some(inline), None) => Some(Prompt::Inline(inline)),
                    (None, Some(path)) => Some(Prompt::FromFile(path)),
                    (None, None) => None,
                    _ => None, // clap enforces exclusivity; this is unreachable
                }
            };

            if count.is_some() && agent.len() > 1 {
                return Err(anyhow!(
                    "--count can only be used with zero or one --agent, but {} were provided",
                    agent.len()
                ));
            }

            let mut tera = Tera::default();
            tera.autoescape_on(vec![]);
            tera.register_filter("slugify", slugify_filter);
            tera.add_raw_template(BRANCH_TEMPLATE_NAME, &branch_template)
                .context("Failed to parse branch template")?;

            // Check if branch_name is a remote ref (e.g., origin/feature/foo)
            let remotes = git::list_remotes().context("Failed to list git remotes")?;
            let detected_remote = remotes
                .iter()
                .find(|r| branch_name.starts_with(&format!("{}/", r)));

            let (remote_branch, template_base_name) = if let Some(remote_name) = detected_remote {
                if base.is_some() {
                    return Err(anyhow!(
                        "Cannot use --base with a remote branch reference. \
                        The remote branch '{}' will be used as the base.",
                        branch_name
                    ));
                }

                let spec = git::parse_remote_branch_spec(&branch_name)
                    .context("Invalid remote branch format. Use <remote>/<branch>")?;

                if spec.remote != *remote_name {
                    return Err(anyhow!("Mismatched remote detection"));
                }

                (Some(branch_name.clone()), spec.branch)
            } else {
                (None, branch_name.clone())
            };

            let resolved_base = if remote_branch.is_some() { None } else { base };

            let cli_default_agent = agent.first().map(|s| s.as_str());
            let config = config::Config::load(cli_default_agent)?;

            let specs = generate_worktree_specs(
                &template_base_name,
                &agent,
                count,
                foreach.as_deref(),
                &tera,
            )?;

            if specs.is_empty() {
                return Err(anyhow!("No worktree specifications were generated"));
            }

            if specs.len() > 1 {
                println!("Preparing to create {} worktrees...", specs.len());
            }

            for (i, spec) in specs.iter().enumerate() {
                if specs.len() > 1 {
                    println!(
                        "\n--- [{}/{}] Creating worktree: {} ---",
                        i + 1,
                        specs.len(),
                        spec.branch_name
                    );
                }

                let prompt_for_spec = if let Some(ref prompt_src) = prompt_template {
                    Some(
                        render_prompt_template(prompt_src, &mut tera, &spec.template_context)
                            .with_context(|| {
                                format!("Failed to render prompt for branch '{}'", spec.branch_name)
                            })?,
                    )
                } else {
                    None
                };

                if options.run_hooks && config.post_create.as_ref().is_some_and(|v| !v.is_empty()) {
                    println!("Running setup commands...");
                }

                let result = workflow::create(
                    &spec.branch_name,
                    resolved_base.as_deref(),
                    remote_branch.as_deref(),
                    prompt_for_spec.as_ref(),
                    &config,
                    options.clone(),
                    spec.agent.as_deref(),
                )
                .with_context(|| {
                    format!(
                        "Failed to create worktree environment for branch '{}'",
                        spec.branch_name
                    )
                })?;

                if result.post_create_hooks_run > 0 {
                    println!("✓ Setup complete");
                }

                println!(
                    "✓ Successfully created worktree and tmux window for '{}'",
                    result.branch_name
                );
                if let Some(ref base) = result.base_branch {
                    println!("  Base: {}", base);
                }
                println!("  Worktree: {}", result.worktree_path.display());
            }

            Ok(())
        }
        Commands::Open {
            branch_name,
            run_hooks,
            force_files,
        } => {
            // Construct setup options (pane commands always run on open)
            let options = SetupOptions::new(run_hooks, force_files, true);
            open_worktree(&branch_name, options)
        }
        Commands::Merge {
            branch_name,
            ignore_uncommitted,
            delete_remote,
            rebase,
            squash,
        } => merge_worktree(
            branch_name.as_deref(),
            ignore_uncommitted,
            delete_remote,
            rebase,
            squash,
        ),
        Commands::Remove(args) => remove_worktree(
            args.branch_name.as_deref(),
            args.force,
            args.delete_remote,
            args.keep_branch,
        ),
        Commands::List => list_worktrees(),
        Commands::Init => config::Config::init(),
        Commands::Claude { command } => match command {
            ClaudeCommands::Prune => prune_claude_config(),
        },
        Commands::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            generate(shell, &mut cmd, name, &mut io::stdout());
            Ok(())
        }
    }
}

fn open_worktree(branch_name: &str, options: SetupOptions) -> Result<()> {
    let config = config::Config::load(None)?;

    if options.run_hooks && config.post_create.as_ref().is_some_and(|v| !v.is_empty()) {
        println!("Running setup commands...");
    }

    let result = workflow::open(branch_name, &config, options)
        .context("Failed to open worktree environment")?;

    if result.post_create_hooks_run > 0 {
        println!("✓ Setup complete");
    }

    println!(
        "✓ Successfully opened tmux window for '{}'\n  Worktree: {}",
        result.branch_name,
        result.worktree_path.display()
    );

    Ok(())
}

fn merge_worktree(
    branch_name: Option<&str>,
    ignore_uncommitted: bool,
    delete_remote: bool,
    rebase: bool,
    squash: bool,
) -> Result<()> {
    let config = config::Config::load(None)?;

    // Determine the branch to merge
    // Note: If running without branch name, we must get current branch BEFORE workflow::merge
    // changes the CWD (since it moves to main worktree for safety)
    let branch_to_merge = if let Some(name) = branch_name {
        name.to_string()
    } else {
        // Running from within a worktree - get current branch
        git::get_current_branch().context("Failed to get current branch")?
    };

    // Print status if there are pre-delete hooks
    if config.pre_delete.as_ref().is_some_and(|v| !v.is_empty()) {
        println!("Running pre-delete commands...");
    }

    let result = workflow::merge(
        Some(&branch_to_merge),
        ignore_uncommitted,
        delete_remote,
        rebase,
        squash,
        &config,
    )
    .context("Failed to merge worktree")?;

    if result.had_staged_changes {
        println!("✓ Committed staged changes");
    }

    println!(
        "Merging '{}' into '{}'...",
        result.branch_merged, result.main_branch
    );
    println!("✓ Merged '{}'", result.branch_merged);

    println!(
        "✓ Successfully merged and cleaned up '{}'",
        result.branch_merged
    );

    Ok(())
}

fn remove_worktree(
    branch_name: Option<&str>,
    mut force: bool,
    delete_remote: bool,
    keep_branch: bool,
) -> Result<()> {
    // Determine the branch to remove
    // Note: If running without branch name, we must get current branch BEFORE workflow::remove
    // changes the CWD (since it moves to main worktree for safety)
    let branch_to_remove = if let Some(name) = branch_name {
        name.to_string()
    } else {
        // Running from within a worktree - get current branch
        git::get_current_branch().context("Failed to get current branch")?
    };

    // Handle user confirmation prompt if needed (before calling workflow)
    if !force {
        // First check for uncommitted changes (must be checked before unmerged prompt)
        // to avoid prompting user about unmerged commits only to error on uncommitted changes
        if let Ok(worktree_path) = git::get_worktree_path(&branch_to_remove)
            && worktree_path.exists()
            && git::has_uncommitted_changes(&worktree_path)?
        {
            return Err(anyhow!(
                "Worktree has uncommitted changes. Use --force to delete anyway."
            ));
        }

        // Check if we need to prompt for unmerged commits (only relevant when deleting the branch)
        if !keep_branch {
            // Try to get the stored base branch, fall back to default branch
            let base = git::get_branch_base(&branch_to_remove)
                .ok()
                .unwrap_or_else(|| {
                    git::get_default_branch().unwrap_or_else(|_| "main".to_string())
                });

            // Get the merge base with fallback if the stored base is invalid
            let base_branch = match git::get_merge_base(&base) {
                Ok(b) => b,
                Err(_) => {
                    let default_main = git::get_default_branch()?;
                    eprintln!(
                        "Warning: Could not resolve base '{}'; falling back to '{}'",
                        base, default_main
                    );
                    git::get_merge_base(&default_main)?
                }
            };

            let unmerged_branches = git::get_unmerged_branches(&base_branch)?;
            let has_unmerged = unmerged_branches.contains(&branch_to_remove);

            if has_unmerged {
                println!(
                    "This will delete the worktree, tmux window, and local branch for '{}'.",
                    branch_to_remove
                );
                if delete_remote {
                    println!("The remote branch will also be deleted.");
                }
                println!(
                    "Warning: Branch '{}' has commits that are not merged into '{}' (base: '{}').",
                    branch_to_remove, base_branch, base
                );
                println!("This action cannot be undone.");
                print!("Are you sure you want to continue? [y/N] ");

                // Flush stdout to ensure the prompt is displayed before reading input
                io::stdout().flush()?;

                let mut confirmation = String::new();
                io::stdin().read_line(&mut confirmation)?;

                if confirmation.trim().to_lowercase() != "y" {
                    println!("Aborted.");
                    return Ok(());
                }

                // User confirmed deletion of unmerged branch - treat as force for git operations
                // This is safe because we already verified there are no uncommitted changes above
                force = true;
            }
        }
    }

    let config = config::Config::load(None)?;

    // Print status if there are pre-delete hooks
    if config.pre_delete.as_ref().is_some_and(|v| !v.is_empty()) {
        println!("Running pre-delete commands...");
    }

    let result = workflow::remove(
        &branch_to_remove,
        force,
        delete_remote,
        keep_branch,
        &config,
    )
    .context("Failed to remove worktree")?;

    if keep_branch {
        println!(
            "✓ Successfully removed worktree for branch '{}'. The local branch was kept.",
            result.branch_removed
        );
    } else {
        println!(
            "✓ Successfully removed worktree and branch '{}'",
            result.branch_removed
        );
    }

    Ok(())
}

fn list_worktrees() -> Result<()> {
    let config = config::Config::load(None)?;
    let worktrees = workflow::list(&config)?;

    if worktrees.is_empty() {
        println!("No worktrees found");
        return Ok(());
    }

    // Prepare display data
    struct DisplayInfo {
        branch: String,
        path_str: String,
        tmux_status: String,
        unmerged_status: String,
    }

    let display_data: Vec<DisplayInfo> = worktrees
        .into_iter()
        .map(|wt| DisplayInfo {
            branch: wt.branch,
            path_str: wt.path.display().to_string(),
            tmux_status: if wt.has_tmux {
                "✓".to_string()
            } else {
                "-".to_string()
            },
            unmerged_status: if wt.has_unmerged {
                "●".to_string()
            } else {
                "-".to_string()
            },
        })
        .collect();

    const BRANCH_HEADER: &str = "BRANCH";
    const TMUX_HEADER: &str = "TMUX";
    const UNMERGED_HEADER: &str = "UNMERGED";
    const PATH_HEADER: &str = "PATH";

    // Determine column widths based on the longest content in each column
    let max_branch_width = display_data
        .iter()
        .map(|wt| wt.branch.len())
        .max()
        .unwrap_or(0)
        .max(BRANCH_HEADER.len());

    // Add padding for visual separation
    let branch_col_width = max_branch_width + 4;
    let tmux_col_width = TMUX_HEADER.len() + 4;
    let unmerged_col_width = UNMERGED_HEADER.len() + 4;

    // Print Header
    println!(
        "{:<branch_width$}{:<tmux_width$}{:<unmerged_width$}{}",
        BRANCH_HEADER,
        TMUX_HEADER,
        UNMERGED_HEADER,
        PATH_HEADER,
        branch_width = branch_col_width,
        tmux_width = tmux_col_width,
        unmerged_width = unmerged_col_width
    );

    // Print Separator
    let branch_separator = "-".repeat(max_branch_width);
    let tmux_separator = "-".repeat(TMUX_HEADER.len());
    let unmerged_separator = "-".repeat(UNMERGED_HEADER.len());
    let path_separator = "-".repeat(PATH_HEADER.len());
    println!(
        "{:<branch_width$}{:<tmux_width$}{:<unmerged_width$}{}",
        branch_separator,
        tmux_separator,
        unmerged_separator,
        path_separator,
        branch_width = branch_col_width,
        tmux_width = tmux_col_width,
        unmerged_width = unmerged_col_width,
    );

    // Print Data Rows
    for wt in display_data {
        println!(
            "{:<branch_width$}{:<tmux_width$}{:<unmerged_width$}{}",
            wt.branch,
            wt.tmux_status,
            wt.unmerged_status,
            wt.path_str,
            branch_width = branch_col_width,
            tmux_width = tmux_col_width,
            unmerged_width = unmerged_col_width
        );
    }

    Ok(())
}

fn prune_claude_config() -> Result<()> {
    claude::prune_stale_entries().context("Failed to prune Claude configuration")?;
    Ok(())
}

fn render_prompt_template(
    prompt: &Prompt,
    tera: &mut Tera,
    context: &TeraContext,
) -> Result<Prompt> {
    let template_str = match prompt {
        Prompt::Inline(text) => text.clone(),
        Prompt::FromFile(path) => fs::read_to_string(path)
            .with_context(|| format!("Failed to read prompt file '{}'", path.display()))?,
    };

    let rendered = tera
        .render_str(&template_str, context)
        .context("Failed to render prompt template")?;
    Ok(Prompt::Inline(rendered))
}

fn generate_worktree_specs(
    base_name: &str,
    agents: &[String],
    count: Option<u32>,
    foreach: Option<&str>,
    tera: &Tera,
) -> Result<Vec<WorktreeSpec>> {
    let is_multi_mode = foreach.is_some() || count.is_some() || agents.len() > 1;

    if !is_multi_mode {
        let agent = agents.first().cloned();
        let mut context = TeraContext::new();
        context.insert("base_name", base_name);
        context.insert("agent", &agent);
        context.insert("num", &Option::<u32>::None);
        context.insert("foreach_vars", &BTreeMap::<String, String>::new());

        return Ok(vec![WorktreeSpec {
            branch_name: base_name.to_string(),
            agent,
            template_context: context,
        }]);
    }

    if let Some(matrix) = foreach {
        let rows = parse_foreach_matrix(matrix)?;
        return rows
            .into_iter()
            .map(|vars| build_spec(tera, base_name, None, None, vars))
            .collect();
    }

    if let Some(times) = count {
        let iterations = times as usize;
        let default_agent = agents.first().cloned();
        let mut specs = Vec::with_capacity(iterations);
        for idx in 0..iterations {
            let num = Some((idx + 1) as u32);
            specs.push(build_spec(
                tera,
                base_name,
                default_agent.clone(),
                num,
                BTreeMap::new(),
            )?);
        }
        return Ok(specs);
    }

    if agents.is_empty() {
        return Ok(vec![build_spec(
            tera,
            base_name,
            None,
            None,
            BTreeMap::new(),
        )?]);
    }

    let mut specs = Vec::with_capacity(agents.len());
    for agent_name in agents {
        specs.push(build_spec(
            tera,
            base_name,
            Some(agent_name.clone()),
            None,
            BTreeMap::new(),
        )?);
    }
    Ok(specs)
}

fn build_spec(
    tera: &Tera,
    base_name: &str,
    agent: Option<String>,
    num: Option<u32>,
    foreach_vars: BTreeMap<String, String>,
) -> Result<WorktreeSpec> {
    let mut context = TeraContext::new();
    context.insert("base_name", base_name);
    context.insert("agent", &agent);
    context.insert("num", &num);
    context.insert("foreach_vars", &foreach_vars);
    for (key, value) in &foreach_vars {
        context.insert(key, value);
    }
    let branch_name = tera
        .render(BRANCH_TEMPLATE_NAME, &context)
        .context("Failed to render branch template")?;
    Ok(WorktreeSpec {
        branch_name,
        agent,
        template_context: context,
    })
}

fn parse_foreach_matrix(input: &str) -> Result<Vec<BTreeMap<String, String>>> {
    let mut columns: Vec<(String, Vec<String>)> = Vec::new();
    let mut seen = HashSet::new();

    for raw in input.split(';') {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }

        let (key, values_str) = trimmed.split_once(':').ok_or_else(|| {
            anyhow!(
                "Invalid --foreach segment '{}'. Use the format name:value1,value2",
                trimmed
            )
        })?;

        let key = key.trim();
        if key.is_empty() {
            return Err(anyhow!(
                "Invalid --foreach segment '{}': variable name cannot be empty",
                trimmed
            ));
        }
        if !seen.insert(key.to_string()) {
            return Err(anyhow!(
                "Duplicate variable '{}' found in --foreach option",
                key
            ));
        }

        let values: Vec<String> = values_str
            .split(',')
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
            .collect();

        if values.is_empty() {
            return Err(anyhow!(
                "Variable '{}' must have at least one value in --foreach",
                key
            ));
        }

        columns.push((key.to_string(), values));
    }

    if columns.is_empty() {
        return Err(anyhow!(
            "--foreach must include at least one variable with values"
        ));
    }

    let expected_len = columns[0].1.len();
    if columns
        .iter()
        .any(|(_, values)| values.len() != expected_len)
    {
        return Err(anyhow!(
            "All --foreach variables must have the same number of values"
        ));
    }

    let mut rows = Vec::with_capacity(expected_len);
    for idx in 0..expected_len {
        let mut map = BTreeMap::new();
        for (key, values) in &columns {
            map.insert(key.clone(), values[idx].clone());
        }
        rows.push(map);
    }

    Ok(rows)
}

fn slugify_filter(value: &Value, _: &HashMap<String, Value>) -> tera::Result<Value> {
    let input = from_value::<String>(value.clone())?;
    let slug = input
        .to_lowercase()
        .chars()
        .map(|c| match c {
            'a'..='z' | '0'..='9' => c,
            _ => '-',
        })
        .collect::<String>()
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    to_value(slug).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tera(template: &str) -> Tera {
        let mut tera = Tera::default();
        tera.autoescape_on(vec![]);
        tera.add_raw_template(BRANCH_TEMPLATE_NAME, template)
            .expect("template should compile");
        tera
    }

    #[test]
    fn parse_foreach_matrix_parses_rows() {
        let rows = parse_foreach_matrix("env:dev,prod;region:us,eu").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("env").unwrap(), "dev");
        assert_eq!(rows[0].get("region").unwrap(), "us");
        assert_eq!(rows[1].get("env").unwrap(), "prod");
        assert_eq!(rows[1].get("region").unwrap(), "eu");
    }

    #[test]
    fn parse_foreach_matrix_requires_matching_lengths() {
        assert!(parse_foreach_matrix("env:dev,prod;region:us").is_err());
    }

    #[test]
    fn generate_specs_with_agents() {
        let tera = create_test_tera("{{ base_name }}{% if agent %}-{{ agent }}{% endif %}");
        let agents = vec!["claude".to_string(), "gemini".to_string()];
        let specs = generate_worktree_specs("feature", &agents, None, None, &tera).expect("specs");
        let summary: Vec<(String, Option<String>)> = specs
            .into_iter()
            .map(|spec| (spec.branch_name, spec.agent))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("feature-claude".to_string(), Some("claude".to_string())),
                ("feature-gemini".to_string(), Some("gemini".to_string()))
            ]
        );
    }

    #[test]
    fn generate_specs_with_count_assigns_numbers() {
        let tera = create_test_tera("{{ base_name }}{% if num %}-{{ num }}{% endif %}");
        let specs = generate_worktree_specs("feature", &[], Some(2), None, &tera).expect("specs");
        let names: Vec<String> = specs.into_iter().map(|s| s.branch_name).collect();
        assert_eq!(
            names,
            vec!["feature-1".to_string(), "feature-2".to_string()]
        );
    }

    #[test]
    fn single_agent_override_preserves_branch_name() {
        let tera = create_test_tera("{{ base_name }}{% if agent %}-{{ agent }}{% endif %}");
        let specs =
            generate_worktree_specs("feature", &[String::from("gemini")], None, None, &tera)
                .expect("specs");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].branch_name, "feature");
        assert_eq!(specs[0].agent.as_deref(), Some("gemini"));
    }

    #[test]
    fn foreach_context_exposes_variables() {
        let mut tera = create_test_tera("{{ base_name }}");
        let specs = generate_worktree_specs(
            "feature",
            &[],
            None,
            Some("platform:ios,android;lang:swift,kotlin"),
            &tera,
        )
        .expect("specs");
        let rendered = tera.render_str("{{ platform }}-{{ lang }}", &specs[0].template_context);
        assert_eq!(rendered.expect("prompt render"), "ios-swift");
    }

    #[test]
    fn render_prompt_template_inline_renders_variables() {
        let mut tera = Tera::default();
        tera.autoescape_on(vec![]);
        let mut context = TeraContext::new();
        context.insert("branch", "feature-123");

        let prompt = Prompt::Inline("Working on {{ branch }}".to_string());
        let result = render_prompt_template(&prompt, &mut tera, &context).expect("render success");

        match result {
            Prompt::Inline(text) => assert_eq!(text, "Working on feature-123"),
            _ => panic!("Expected Inline prompt"),
        }
    }

    #[test]
    fn render_prompt_template_from_file_reads_and_renders() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let mut tera = Tera::default();
        tera.autoescape_on(vec![]);
        let mut context = TeraContext::new();
        context.insert("name", "test-branch");

        let mut temp_file = NamedTempFile::new().expect("create temp file");
        writeln!(temp_file, "Branch: {{{{ name }}}}").expect("write to temp file");
        let temp_path = temp_file.path().to_path_buf();

        let prompt = Prompt::FromFile(temp_path);
        let result = render_prompt_template(&prompt, &mut tera, &context).expect("render success");

        match result {
            Prompt::Inline(text) => assert_eq!(text, "Branch: test-branch\n"),
            _ => panic!("Expected Inline prompt"),
        }
    }

    #[test]
    fn render_prompt_template_from_nonexistent_file_fails() {
        let mut tera = Tera::default();
        let context = TeraContext::new();

        let prompt = Prompt::FromFile(PathBuf::from("/nonexistent/path/to/file.txt"));
        let result = render_prompt_template(&prompt, &mut tera, &context);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to read prompt file")
        );
    }
}
