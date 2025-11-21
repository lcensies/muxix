use crate::workflow::SetupOptions;
use crate::{claude, config, git, workflow};
use anyhow::{Context, Result, anyhow};
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Shell, generate};
use edit::Builder;
use minijinja::{AutoEscape, Environment};
use serde::Deserialize;
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Write};
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

#[derive(Debug, Deserialize, Default)]
struct PromptMetadata {
    #[serde(default)]
    foreach: Option<BTreeMap<String, Vec<String>>>,
}

#[derive(Debug)]
struct PromptDocument {
    body: String,
    meta: PromptMetadata,
}

#[derive(Debug, Clone)]
struct WorktreeSpec {
    branch_name: String,
    agent: Option<String>,
    template_context: JsonValue,
}

type TemplateEnv = Environment<'static>;

/// Reserved template variable names that cannot be used in foreach
const RESERVED_TEMPLATE_KEYS: &[&str] = &["base_name", "agent", "num", "foreach_vars"];

#[derive(clap::Args, Debug)]
struct PromptArgs {
    /// Inline prompt text to store in the new worktree
    #[arg(short = 'p', long, conflicts_with_all = ["prompt_file", "prompt_editor"])]
    prompt: Option<String>,

    /// Path to a file whose contents should be used as the prompt
    #[arg(short = 'P', long = "prompt-file", conflicts_with_all = ["prompt", "prompt_editor"])]
    prompt_file: Option<PathBuf>,

    /// Open $EDITOR to write the prompt
    #[arg(short = 'e', long = "prompt-editor", conflicts_with_all = ["prompt", "prompt_file"])]
    prompt_editor: bool,
}

#[derive(clap::Args, Debug)]
struct SetupFlags {
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
}

#[derive(clap::Args, Debug)]
struct MultiArgs {
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
        default_value = r#"{{ base_name }}{% if agent %}-{{ agent | slugify }}{% endif %}{% for key in foreach_vars %}-{{ foreach_vars[key] | slugify }}{% endfor %}{% if num %}-{{ num }}{% endif %}"#
    )]
    branch_template: String,
}

#[derive(clap::Args, Debug)]
struct RescueArgs {
    /// Move uncommitted changes from the current worktree to the new worktree
    #[arg(short = 'w', long, conflicts_with_all = ["count", "foreach"])]
    with_changes: bool,

    /// Interactively select which changes to move (only applies with --with-changes)
    #[arg(long, requires = "with_changes")]
    patch: bool,

    /// Also move untracked files (only applies with --with-changes)
    #[arg(short = 'u', long, requires = "with_changes")]
    include_untracked: bool,
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

        #[command(flatten)]
        prompt: PromptArgs,

        #[command(flatten)]
        setup: SetupFlags,

        #[command(flatten)]
        rescue: RescueArgs,

        #[command(flatten)]
        multi: MultiArgs,
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
    Remove {
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
    },

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
            setup,
            rescue,
            multi,
        } => add_worktree(&branch_name, base.as_deref(), prompt, setup, rescue, multi),
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
        Commands::Remove {
            branch_name,
            force,
            delete_remote,
            keep_branch,
        } => remove_worktree(branch_name.as_deref(), force, delete_remote, keep_branch),
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

fn add_worktree(
    branch_name: &str,
    base: Option<&str>,
    prompt_args: PromptArgs,
    setup: SetupFlags,
    rescue: RescueArgs,
    multi: MultiArgs,
) -> Result<()> {
    // If --with-changes is set, use the create_with_changes workflow
    if rescue.with_changes {
        let mut options =
            SetupOptions::new(!setup.no_hooks, !setup.no_file_ops, !setup.no_pane_cmds);
        options.focus_window = !setup.background;
        let config = config::Config::load(None)?;
        let result = workflow::create_with_changes(
            branch_name,
            rescue.include_untracked,
            rescue.patch,
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
    let mut options = SetupOptions::new(!setup.no_hooks, !setup.no_file_ops, !setup.no_pane_cmds);
    options.focus_window = !setup.background;

    let prompt_template = if prompt_args.prompt_editor {
        let mut builder = Builder::new();
        builder.suffix(".md");
        let editor_content = edit::edit_with_builder("", &builder)
            .context("Failed to open editor or read content")?;
        let trimmed = editor_content.trim();
        if trimmed.is_empty() {
            return Err(anyhow!("Aborting: prompt is empty"));
        }
        Some(Prompt::Inline(trimmed.to_string()))
    } else {
        match (prompt_args.prompt, prompt_args.prompt_file) {
            (Some(inline), None) => Some(Prompt::Inline(inline)),
            (None, Some(path)) => Some(Prompt::FromFile(path)),
            (None, None) => None,
            _ => None, // clap enforces exclusivity; this is unreachable
        }
    };

    // Parse prompt document to extract frontmatter (if applicable)
    let prompt_doc = if let Some(ref prompt_src) = prompt_template {
        // Parse frontmatter from file or editor content, but skip for inline prompts
        // that didn't come from the editor (those are pure strings from -p flag)
        let should_parse_frontmatter =
            prompt_args.prompt_editor || matches!(prompt_src, Prompt::FromFile(_));

        if should_parse_frontmatter {
            Some(parse_prompt_document(prompt_src)?)
        } else {
            // Inline prompt without editor: no frontmatter parsing
            Some(PromptDocument {
                body: match prompt_src {
                    Prompt::Inline(s) => s.clone(),
                    Prompt::FromFile(_) => unreachable!(),
                },
                meta: PromptMetadata::default(),
            })
        }
    } else {
        None
    };

    if multi.count.is_some() && multi.agent.len() > 1 {
        return Err(anyhow!(
            "--count can only be used with zero or one --agent, but {} were provided",
            multi.agent.len()
        ));
    }

    // Validate that --agent CLI flag and frontmatter foreach are not both present
    let has_foreach_in_prompt = prompt_doc
        .as_ref()
        .and_then(|d| d.meta.foreach.as_ref())
        .is_some();

    if has_foreach_in_prompt && !multi.agent.is_empty() {
        return Err(anyhow!(
            "Cannot use --agent when 'foreach' is defined in the prompt frontmatter. \
            These multi-worktree generation methods are mutually exclusive."
        ));
    }

    let mut env = Environment::new();
    env.set_auto_escape_callback(|_| AutoEscape::None);
    env.set_keep_trailing_newline(true);
    env.add_filter("slugify", slugify_filter);

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

        let spec = git::parse_remote_branch_spec(branch_name)
            .context("Invalid remote branch format. Use <remote>/<branch>")?;

        if spec.remote != *remote_name {
            return Err(anyhow!("Mismatched remote detection"));
        }

        (Some(branch_name.to_string()), spec.branch)
    } else {
        (None, branch_name.to_string())
    };

    let resolved_base = if remote_branch.is_some() { None } else { base };

    let cli_default_agent = multi.agent.first().map(|s| s.as_str());
    let config = config::Config::load(cli_default_agent)?;

    // Determine effective foreach matrix: CLI overrides frontmatter
    let effective_foreach_rows = match (
        &multi.foreach,
        prompt_doc.as_ref().and_then(|d| d.meta.foreach.as_ref()),
    ) {
        (Some(cli_str), Some(_frontmatter_map)) => {
            eprintln!("Warning: --foreach overrides prompt frontmatter");
            Some(parse_foreach_matrix(cli_str)?)
        }
        (Some(cli_str), None) => Some(parse_foreach_matrix(cli_str)?),
        (None, Some(frontmatter_map)) => Some(foreach_from_frontmatter(frontmatter_map)?),
        (None, None) => None,
    };

    let specs = generate_worktree_specs(
        &template_base_name,
        &multi.agent,
        multi.count,
        effective_foreach_rows.as_deref(),
        &env,
        &multi.branch_template,
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

        let prompt_for_spec = if let Some(ref doc) = prompt_doc {
            Some(Prompt::Inline(
                render_prompt_body(&doc.body, &env, &spec.template_context).with_context(|| {
                    format!("Failed to render prompt for branch '{}'", spec.branch_name)
                })?,
            ))
        } else {
            None
        };

        if options.run_hooks && config.post_create.as_ref().is_some_and(|v| !v.is_empty()) {
            println!("Running setup commands...");
        }

        let result = workflow::create(
            &spec.branch_name,
            resolved_base,
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

#[cfg(test)]
fn render_prompt_template(
    prompt: &Prompt,
    env: &TemplateEnv,
    context: &JsonValue,
) -> Result<Prompt> {
    let template_str = match prompt {
        Prompt::Inline(text) => text.clone(),
        Prompt::FromFile(path) => fs::read_to_string(path)
            .with_context(|| format!("Failed to read prompt file '{}'", path.display()))?,
    };

    let rendered = env
        .render_str(&template_str, context)
        .context("Failed to render prompt template")?;
    Ok(Prompt::Inline(rendered))
}

/// Render a prompt body string with the given template context.
fn render_prompt_body(body: &str, env: &TemplateEnv, context: &JsonValue) -> Result<String> {
    env.render_str(body, context)
        .context("Failed to render prompt template")
}

fn generate_worktree_specs(
    base_name: &str,
    agents: &[String],
    count: Option<u32>,
    foreach_rows: Option<&[BTreeMap<String, String>]>,
    env: &TemplateEnv,
    branch_template: &str,
) -> Result<Vec<WorktreeSpec>> {
    let is_multi_mode = foreach_rows.is_some() || count.is_some() || agents.len() > 1;

    if !is_multi_mode {
        let agent = agents.first().cloned();
        let num: Option<u32> = None;
        let foreach_vars = BTreeMap::<String, String>::new();
        let context = build_template_context(base_name, &agent, &num, &foreach_vars);

        // Intentional: in single-agent/instance mode the CLI keeps the provided
        // branch name verbatim so users can opt into templating only when they
        // request multiple worktrees.
        return Ok(vec![WorktreeSpec {
            branch_name: base_name.to_string(),
            agent,
            template_context: context,
        }]);
    }

    if let Some(rows) = foreach_rows {
        return rows
            .iter()
            .map(|vars| build_spec(env, branch_template, base_name, None, None, vars.clone()))
            .collect();
    }

    if let Some(times) = count {
        let iterations = times as usize;
        let default_agent = agents.first().cloned();
        let mut specs = Vec::with_capacity(iterations);
        for idx in 0..iterations {
            let num = Some((idx + 1) as u32);
            specs.push(build_spec(
                env,
                branch_template,
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
            env,
            branch_template,
            base_name,
            None,
            None,
            BTreeMap::new(),
        )?]);
    }

    let mut specs = Vec::with_capacity(agents.len());
    for agent_name in agents {
        specs.push(build_spec(
            env,
            branch_template,
            base_name,
            Some(agent_name.clone()),
            None,
            BTreeMap::new(),
        )?);
    }
    Ok(specs)
}

fn build_spec(
    env: &TemplateEnv,
    branch_template: &str,
    base_name: &str,
    agent: Option<String>,
    num: Option<u32>,
    foreach_vars: BTreeMap<String, String>,
) -> Result<WorktreeSpec> {
    // Extract agent from foreach_vars if present (treat "agent" as a special reserved key)
    let effective_agent = agent.or_else(|| foreach_vars.get("agent").cloned());

    let context = build_template_context(base_name, &effective_agent, &num, &foreach_vars);
    let branch_name = env
        .render_str(branch_template, &context)
        .context("Failed to render branch template")?;
    Ok(WorktreeSpec {
        branch_name,
        agent: effective_agent,
        template_context: context,
    })
}

fn build_template_context(
    base_name: &str,
    agent: &Option<String>,
    num: &Option<u32>,
    foreach_vars: &BTreeMap<String, String>,
) -> JsonValue {
    let mut context = JsonMap::new();
    context.insert(
        "base_name".to_string(),
        JsonValue::String(base_name.to_string()),
    );

    let agent_value = agent
        .as_ref()
        .map(|value| JsonValue::String(value.clone()))
        .unwrap_or(JsonValue::Null);
    context.insert("agent".to_string(), agent_value);

    let num_value = num
        .as_ref()
        .map(|value| JsonValue::Number(JsonNumber::from(*value)))
        .unwrap_or(JsonValue::Null);
    context.insert("num".to_string(), num_value);

    let mut foreach_json = JsonMap::new();
    for (key, value) in foreach_vars {
        // Filter out ALL reserved keys to avoid collisions in templates
        // Reserved keys: base_name, agent, num, foreach_vars
        if !RESERVED_TEMPLATE_KEYS.contains(&key.as_str()) {
            foreach_json.insert(key.clone(), JsonValue::String(value.clone()));
            context.insert(key.clone(), JsonValue::String(value.clone()));
        }
    }
    context.insert("foreach_vars".to_string(), JsonValue::Object(foreach_json));

    JsonValue::Object(context)
}

/// Split frontmatter from markdown content.
/// Returns (Some(frontmatter_yaml), body) if frontmatter exists, or (None, content) if not.
fn split_frontmatter(content: &str) -> (Option<String>, &str) {
    let lines: Vec<&str> = content.lines().collect();

    // Check if content starts with "---"
    if lines.is_empty() || lines[0].trim() != "---" {
        return (None, content);
    }

    // Find the closing "---" or "..."
    let closing_idx = lines.iter().skip(1).position(|line| {
        let trimmed = line.trim();
        trimmed == "---" || trimmed == "..."
    });

    match closing_idx {
        Some(idx) => {
            // closing_idx is relative to skip(1), so actual index is idx + 1
            let actual_idx = idx + 1;
            let frontmatter = lines[1..actual_idx].join("\n");
            // Body starts after the closing fence
            let body_start = lines
                .iter()
                .take(actual_idx + 1)
                .map(|l| l.len() + 1)
                .sum::<usize>();
            let body = &content[body_start.min(content.len())..];
            (Some(frontmatter), body)
        }
        None => {
            // No closing fence found, treat entire content as body
            (None, content)
        }
    }
}

/// Parse a prompt document, extracting frontmatter metadata and body.
fn parse_prompt_document(prompt: &Prompt) -> Result<PromptDocument> {
    // Store the file content to avoid dangling reference
    let content_storage: String;
    let content = match prompt {
        Prompt::Inline(text) => text.as_str(),
        Prompt::FromFile(path) => {
            content_storage = fs::read_to_string(path)
                .with_context(|| format!("Failed to read prompt file: {}", path.display()))?;
            &content_storage
        }
    };

    let (frontmatter_yaml, body) = split_frontmatter(content);

    let meta = if let Some(ref yaml) = frontmatter_yaml {
        serde_yaml::from_str(yaml).context("Failed to parse YAML frontmatter")?
    } else {
        PromptMetadata::default()
    };

    Ok(PromptDocument {
        body: body.to_string(),
        meta,
    })
}

/// Convert frontmatter foreach (BTreeMap<String, Vec<String>>) to matrix rows.
/// Validates that all value lists have equal length (zip constraint).
fn foreach_from_frontmatter(
    foreach_map: &BTreeMap<String, Vec<String>>,
) -> Result<Vec<BTreeMap<String, String>>> {
    if foreach_map.is_empty() {
        return Err(anyhow!(
            "foreach in frontmatter must include at least one variable"
        ));
    }

    // Get the first list length as reference
    let expected_len = foreach_map.values().next().unwrap().len();

    if expected_len == 0 {
        return Err(anyhow!("foreach variables must have at least one value"));
    }

    // Validate all lists have the same length
    for (key, values) in foreach_map.iter() {
        if values.len() != expected_len {
            return Err(anyhow!(
                "All foreach variables must have the same number of values (expected {}, but '{}' has {})",
                expected_len,
                key,
                values.len()
            ));
        }
    }

    // Zip values by index to create row dictionaries
    let mut rows = Vec::with_capacity(expected_len);
    for idx in 0..expected_len {
        let mut row = BTreeMap::new();
        for (key, values) in foreach_map {
            row.insert(key.clone(), values[idx].clone());
        }
        rows.push(row);
    }

    Ok(rows)
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

fn slugify_filter(input: String) -> String {
    input
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
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_env() -> TemplateEnv {
        let mut env = Environment::new();
        env.set_auto_escape_callback(|_| AutoEscape::None);
        env.set_keep_trailing_newline(true);
        env.add_filter("slugify", slugify_filter);
        env
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
        let env = create_test_env();
        let agents = vec!["claude".to_string(), "gemini".to_string()];
        let specs = generate_worktree_specs(
            "feature",
            &agents,
            None,
            None,
            &env,
            "{{ base_name }}{% if agent %}-{{ agent }}{% endif %}",
        )
        .expect("specs");
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
        let env = create_test_env();
        let specs = generate_worktree_specs(
            "feature",
            &[],
            Some(2),
            None,
            &env,
            "{{ base_name }}{% if num %}-{{ num }}{% endif %}",
        )
        .expect("specs");
        let names: Vec<String> = specs.into_iter().map(|s| s.branch_name).collect();
        assert_eq!(
            names,
            vec!["feature-1".to_string(), "feature-2".to_string()]
        );
    }

    #[test]
    fn single_agent_override_preserves_branch_name() {
        let env = create_test_env();
        let specs = generate_worktree_specs(
            "feature",
            &[String::from("gemini")],
            None,
            None,
            &env,
            "{{ base_name }}{% if agent %}-{{ agent }}{% endif %}",
        )
        .expect("specs");
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].branch_name, "feature");
        assert_eq!(specs[0].agent.as_deref(), Some("gemini"));
    }

    #[test]
    fn foreach_context_exposes_variables() {
        let env = create_test_env();
        let rows = parse_foreach_matrix("platform:ios,android;lang:swift,kotlin").expect("parse");
        let specs =
            generate_worktree_specs("feature", &[], None, Some(&rows), &env, "{{ base_name }}")
                .expect("specs");
        let rendered = env
            .render_str("{{ platform }}-{{ lang }}", &specs[0].template_context)
            .expect("prompt render");
        assert_eq!(rendered, "ios-swift");
    }

    #[test]
    fn render_prompt_template_inline_renders_variables() {
        let env = create_test_env();
        let mut context_map = JsonMap::new();
        context_map.insert(
            "branch".to_string(),
            JsonValue::String("feature-123".to_string()),
        );
        let context = JsonValue::Object(context_map);

        let prompt = Prompt::Inline("Working on {{ branch }}".to_string());
        let result = render_prompt_template(&prompt, &env, &context).expect("render success");

        match result {
            Prompt::Inline(text) => assert_eq!(text, "Working on feature-123"),
            _ => panic!("Expected Inline prompt"),
        }
    }

    #[test]
    fn render_prompt_template_from_file_reads_and_renders() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let env = create_test_env();
        let mut context_map = JsonMap::new();
        context_map.insert(
            "name".to_string(),
            JsonValue::String("test-branch".to_string()),
        );
        let context = JsonValue::Object(context_map);

        let mut temp_file = NamedTempFile::new().expect("create temp file");
        writeln!(temp_file, "Branch: {{{{ name }}}}").expect("write to temp file");
        let temp_path = temp_file.path().to_path_buf();

        let prompt = Prompt::FromFile(temp_path);
        let result = render_prompt_template(&prompt, &env, &context).expect("render success");

        match result {
            Prompt::Inline(text) => assert_eq!(text, "Branch: test-branch\n"),
            _ => panic!("Expected Inline prompt"),
        }
    }

    #[test]
    fn render_prompt_template_from_nonexistent_file_fails() {
        let env = create_test_env();
        let context = JsonValue::Null;

        let prompt = Prompt::FromFile(PathBuf::from("/nonexistent/path/to/file.txt"));
        let result = render_prompt_template(&prompt, &env, &context);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to read prompt file")
        );
    }

    #[test]
    fn split_frontmatter_extracts_yaml_and_body() {
        let content = "---\nkey: value\n---\n\nBody content here";
        let (frontmatter, body) = split_frontmatter(content);

        assert_eq!(frontmatter, Some("key: value".to_string()));
        assert_eq!(body, "\nBody content here");
    }

    #[test]
    fn split_frontmatter_handles_no_frontmatter() {
        let content = "Just body content";
        let (frontmatter, body) = split_frontmatter(content);

        assert_eq!(frontmatter, None);
        assert_eq!(body, "Just body content");
    }

    #[test]
    fn split_frontmatter_handles_missing_closing_fence() {
        let content = "---\nkey: value\nno closing fence";
        let (frontmatter, body) = split_frontmatter(content);

        assert_eq!(frontmatter, None);
        assert_eq!(body, "---\nkey: value\nno closing fence");
    }

    #[test]
    fn parse_prompt_document_with_frontmatter() {
        let content = "---\nforeach:\n  platform: [iOS, Android]\n---\n\nBuild for {{ platform }}";
        let prompt = Prompt::Inline(content.to_string());
        let doc = parse_prompt_document(&prompt).expect("parse success");

        assert_eq!(doc.body, "\nBuild for {{ platform }}");
        assert!(doc.meta.foreach.is_some());

        let foreach = doc.meta.foreach.unwrap();
        assert_eq!(
            foreach.get("platform").unwrap(),
            &vec!["iOS".to_string(), "Android".to_string()]
        );
    }

    #[test]
    fn parse_prompt_document_without_frontmatter() {
        let content = "Build for {{ platform }}";
        let prompt = Prompt::Inline(content.to_string());
        let doc = parse_prompt_document(&prompt).expect("parse success");

        assert_eq!(doc.body, "Build for {{ platform }}");
        assert!(doc.meta.foreach.is_none());
    }

    #[test]
    fn foreach_from_frontmatter_creates_rows() {
        let mut map = BTreeMap::new();
        map.insert(
            "platform".to_string(),
            vec!["iOS".to_string(), "Android".to_string()],
        );
        map.insert(
            "lang".to_string(),
            vec!["swift".to_string(), "kotlin".to_string()],
        );

        let rows = foreach_from_frontmatter(&map).expect("conversion success");

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("platform").unwrap(), "iOS");
        assert_eq!(rows[0].get("lang").unwrap(), "swift");
        assert_eq!(rows[1].get("platform").unwrap(), "Android");
        assert_eq!(rows[1].get("lang").unwrap(), "kotlin");
    }

    #[test]
    fn foreach_from_frontmatter_requires_equal_lengths() {
        let mut map = BTreeMap::new();
        map.insert(
            "platform".to_string(),
            vec!["iOS".to_string(), "Android".to_string()],
        );
        map.insert("lang".to_string(), vec!["swift".to_string()]);

        let result = foreach_from_frontmatter(&map);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("same number of values")
        );
    }

    #[test]
    fn foreach_from_frontmatter_rejects_empty_values() {
        let mut map = BTreeMap::new();
        map.insert("platform".to_string(), vec![]);

        let result = foreach_from_frontmatter(&map);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("at least one value")
        );
    }

    #[test]
    fn branch_template_renders_with_foreach_vars() {
        let env = create_test_env();
        let mut foreach_vars = BTreeMap::new();
        foreach_vars.insert("platform".to_string(), "ios".to_string());
        foreach_vars.insert("lang".to_string(), "swift".to_string());

        let context = build_template_context("feature", &None, &None, &foreach_vars);
        // MiniJinja doesn't support unpacking in for loops, so we iterate over keys
        let template = "{{ base_name }}{% for key in foreach_vars %}-{{ foreach_vars[key] | slugify }}{% endfor %}";
        let result = env.render_str(template, &context).expect("render");

        // The foreach_vars iteration should include both platform and lang values
        // BTreeMap is sorted, so lang comes before platform alphabetically
        assert_eq!(result, "feature-swift-ios");
    }

    #[test]
    fn parse_prompt_document_from_file_with_frontmatter() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let content = "---\nforeach:\n  platform: [iOS, Android]\n  lang: [swift, kotlin]\n---\n\nBuild for {{ platform }} using {{ lang }}";
        let mut temp_file = NamedTempFile::new().expect("create temp file");
        write!(temp_file, "{}", content).expect("write to temp file");
        let temp_path = temp_file.path().to_path_buf();

        let prompt = Prompt::FromFile(temp_path);
        let doc = parse_prompt_document(&prompt).expect("parse success");

        assert_eq!(doc.body, "\nBuild for {{ platform }} using {{ lang }}");
        assert!(doc.meta.foreach.is_some());

        let foreach = doc.meta.foreach.unwrap();
        assert_eq!(foreach.len(), 2);
        assert_eq!(
            foreach.get("platform").unwrap(),
            &vec!["iOS".to_string(), "Android".to_string()]
        );
        assert_eq!(
            foreach.get("lang").unwrap(),
            &vec!["swift".to_string(), "kotlin".to_string()]
        );
    }

    #[test]
    fn parse_prompt_document_from_file_without_frontmatter() {
        use std::io::Write;
        use tempfile::NamedTempFile;

        let content = "Build for {{ platform }}";
        let mut temp_file = NamedTempFile::new().expect("create temp file");
        write!(temp_file, "{}", content).expect("write to temp file");
        let temp_path = temp_file.path().to_path_buf();

        let prompt = Prompt::FromFile(temp_path);
        let doc = parse_prompt_document(&prompt).expect("parse success");

        assert_eq!(doc.body, "Build for {{ platform }}");
        assert!(doc.meta.foreach.is_none());
    }

    #[test]
    fn foreach_with_agent_key_populates_spec_agent() {
        let env = create_test_env();
        let mut map = BTreeMap::new();
        map.insert(
            "agent".to_string(),
            vec!["claude".to_string(), "gemini".to_string()],
        );

        let rows = foreach_from_frontmatter(&map).expect("conversion success");
        let specs = generate_worktree_specs(
            "feature",
            &[],
            None,
            Some(&rows),
            &env,
            "{{ base_name }}{% if agent %}-{{ agent | slugify }}{% endif %}{% for key in foreach_vars %}-{{ foreach_vars[key] | slugify }}{% endfor %}",
        )
        .expect("specs");

        assert_eq!(specs.len(), 2);

        // First spec should have agent=claude and branch name should NOT include agent twice
        assert_eq!(specs[0].branch_name, "feature-claude");
        assert_eq!(specs[0].agent.as_deref(), Some("claude"));

        // Second spec should have agent=gemini
        assert_eq!(specs[1].branch_name, "feature-gemini");
        assert_eq!(specs[1].agent.as_deref(), Some("gemini"));
    }

    #[test]
    fn foreach_with_agent_and_other_vars_filters_agent_from_iteration() {
        let env = create_test_env();
        let mut map = BTreeMap::new();
        map.insert(
            "agent".to_string(),
            vec!["claude".to_string(), "gemini".to_string()],
        );
        map.insert(
            "platform".to_string(),
            vec!["ios".to_string(), "android".to_string()],
        );

        let rows = foreach_from_frontmatter(&map).expect("conversion success");
        let specs = generate_worktree_specs(
            "feature",
            &[],
            None,
            Some(&rows),
            &env,
            "{{ base_name }}{% if agent %}-{{ agent | slugify }}{% endif %}{% for key in foreach_vars %}-{{ foreach_vars[key] | slugify }}{% endfor %}",
        )
        .expect("specs");

        assert_eq!(specs.len(), 2);

        // Branch names should be: base-agent-platform (NOT base-agent-agent-platform or base-agent-platform-agent)
        // BTreeMap is sorted, so "agent" comes before "platform" alphabetically, but agent is filtered from foreach_vars
        assert_eq!(specs[0].branch_name, "feature-claude-ios");
        assert_eq!(specs[0].agent.as_deref(), Some("claude"));

        assert_eq!(specs[1].branch_name, "feature-gemini-android");
        assert_eq!(specs[1].agent.as_deref(), Some("gemini"));
    }

    #[test]
    fn foreach_filters_all_reserved_keys() {
        let env = create_test_env();
        let mut map = BTreeMap::new();
        // Try to use reserved keys in foreach
        map.insert(
            "base_name".to_string(),
            vec!["bad1".to_string(), "bad2".to_string()],
        );
        map.insert(
            "num".to_string(),
            vec!["bad3".to_string(), "bad4".to_string()],
        );
        map.insert(
            "foreach_vars".to_string(),
            vec!["bad5".to_string(), "bad6".to_string()],
        );
        map.insert(
            "agent".to_string(),
            vec!["bad7".to_string(), "bad8".to_string()],
        );
        map.insert(
            "platform".to_string(),
            vec!["ios".to_string(), "android".to_string()],
        );

        let rows = foreach_from_frontmatter(&map).expect("conversion success");

        // Verify that reserved keys are NOT in the rows at the top level (only in the BTreeMap for lookup)
        // But the row itself should still contain them for extraction
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("platform").unwrap(), "ios");
        assert_eq!(rows[1].get("platform").unwrap(), "android");

        let specs = generate_worktree_specs(
            "base",
            &[],
            None,
            Some(&rows),
            &env,
            "{{ base_name }}{% if agent %}-{{ agent | slugify }}{% endif %}{% for key in foreach_vars %}-{{ foreach_vars[key] | slugify }}{% endfor %}",
        )
        .expect("specs");

        // Branch name should only include platform, not the reserved keys
        // Reserved keys should be filtered from foreach_vars iteration
        assert_eq!(specs[0].branch_name, "base-bad7-ios");
        assert_eq!(specs[1].branch_name, "base-bad8-android");

        // base_name should be "base" (from function param), not "bad1" or "bad2"
        let context0 = &specs[0].template_context;
        assert_eq!(context0["base_name"].as_str().unwrap(), "base");

        // agent should be from foreach (bad7/bad8), not overwritten by reserved key collision
        assert_eq!(context0["agent"].as_str().unwrap(), "bad7");
    }
}
