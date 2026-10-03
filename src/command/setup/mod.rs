use anyhow::Result;
use console::style;
use std::io::{self, IsTerminal, Write};

use crate::agent::setup;
use crate::config::Config;

pub mod agent_hooks;
pub mod managed;
pub mod report;
pub mod sections;

#[cfg(test)]
mod tests;

pub use report::{ItemResult, Outcome, Section, SetupReport};

/// How a `workmux setup` invocation should behave.
#[derive(Debug, Clone, Default)]
pub struct SetupOptions {
    /// Apply every section without prompting.
    pub non_interactive: bool,
    /// Compute what would change, write nothing, exit 2 on drift.
    pub check: bool,
    /// Emit one JSON object on stdout; human output goes to stderr.
    pub json: bool,
    /// Restrict to these sections. Empty means all of them.
    pub only: Vec<Section>,
    /// Leave harness features the config no longer declares installed.
    pub no_prune: bool,
    /// Config profile(s) to resolve with.
    pub profile: Option<String>,
}

impl SetupOptions {
    /// Sections to run, in the canonical order.
    fn selected(&self) -> Vec<Section> {
        if self.only.is_empty() {
            Section::ALL.to_vec()
        } else {
            Section::ALL
                .iter()
                .copied()
                .filter(|s| self.only.contains(s))
                .collect()
        }
    }

    /// Whether this run bypasses the interactive prompt flow.
    pub fn is_automated(&self) -> bool {
        self.non_interactive || self.check || self.json
    }
}

/// Parse a comma-separated `--only` list.
pub fn parse_sections(raw: &str) -> Result<Vec<Section>> {
    let mut out = Vec::new();
    for name in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match Section::parse(name) {
            Some(s) => out.push(s),
            None => anyhow::bail!(
                "unknown setup section `{name}`; valid sections: {}",
                Section::all_names()
            ),
        }
    }
    Ok(out)
}

/// Shared prelude for both setup entry points: profile-aware config load,
/// agent probing, and project root resolution. Keeps `run_automated` and the
/// interactive `run` on one code path.
struct Prepared {
    config: Option<Config>,
    checks: Vec<setup::AgentCheck>,
    project_root: std::path::PathBuf,
}

fn prepare(profile: Option<&str>) -> Prepared {
    let config = Config::load_with_options(
        &std::env::current_dir().unwrap_or_default(),
        None,
        None,
        profile,
    )
    .map(|(c, _)| c)
    .ok();

    let checks = setup::check_all();
    let project_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    Prepared {
        config,
        checks,
        project_root,
    }
}

const NO_AGENTS_MSG: &str =
    "No agents detected. Install an agent CLI (Claude Code, OpenCode) to get started.";

/// Entry point for automated runs: `--non-interactive`, `--check`, `--json`.
///
/// Returns the process exit code rather than erroring on item failures, so a
/// partial failure still reports every other section's result.
pub fn run_automated(opts: &SetupOptions) -> Result<i32> {
    let Prepared {
        config,
        checks,
        project_root,
    } = prepare(opts.profile.as_deref());

    if checks.is_empty() {
        if opts.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&SetupReport::default().to_json())?
            );
            eprintln!("{NO_AGENTS_MSG}");
        } else {
            println!("{NO_AGENTS_MSG}");
        }
        return Ok(0);
    }

    let report = sections::run_all_with_prune(
        &opts.selected(),
        &checks,
        config.as_ref(),
        &project_root,
        opts.check,
        !opts.no_prune,
    );

    if opts.json {
        println!("{}", serde_json::to_string_pretty(&report.to_json())?);
    } else {
        print_report(&report);
    }

    Ok(report.exit_code())
}

/// Render a report as human-readable lines.
///
/// Goes to stderr under `--json` so stdout stays a single parseable object.
fn print_report(report: &SetupReport) {
    let out = |line: String| println!("{line}");

    if report.is_empty() {
        out("Nothing to apply.".to_string());
        return;
    }

    let mut last_section = None;
    for item in &report.items {
        if last_section != Some(item.section) {
            out(format!("\n  {}", style(item.section.as_str()).bold().cyan()));
            last_section = Some(item.section);
        }
        let mark = match item.outcome {
            Outcome::Installed | Outcome::Updated => style("✓").green(),
            Outcome::Removed => style("✗").yellow(),
            Outcome::UpToDate => style("=").dim(),
            Outcome::Skipped => style("-").dim(),
            Outcome::Failed => style("✗").red(),
        };
        let agent = item.agent.as_deref().unwrap_or("-");
        let detail = item
            .detail
            .as_deref()
            .map(|d| format!(" ({d})"))
            .unwrap_or_default();
        out(format!(
            "    {mark} {agent}: {} [{}]{detail}",
            item.name, item.outcome
        ));
    }

    if report.check_only {
        let drifted = report.drifted();
        out(String::new());
        if drifted.is_empty() {
            out(format!("  {}", style("in sync").green()));
        } else {
            out(format!(
                "  {} {} item(s) would change",
                style("drift:").yellow(),
                drifted.len()
            ));
        }
    }
}

pub fn run(opts: &SetupOptions) -> Result<()> {
    if !io::stdin().is_terminal() {
        anyhow::bail!(
            "workmux setup requires an interactive terminal; pass --non-interactive to apply \
             without prompting"
        );
    }

    let Prepared {
        config,
        checks,
        project_root,
    } = prepare(opts.profile.as_deref());

    if checks.is_empty() {
        println!("{NO_AGENTS_MSG}");
        return Ok(());
    }

    let selected = opts.selected();

    let preview = sections::run_all_with_prune(
        &selected,
        &checks,
        config.as_ref(),
        &project_root,
        true,
        !opts.no_prune,
    );
    print_report(&preview);

    // Prompts and MCP sync have no read-back, so `--check` reports them as
    // skipped; only the apply pass can converge them.
    let has_skipped = preview.items.iter().any(|i| i.outcome == Outcome::Skipped);
    let drifted = !preview.drifted().is_empty();
    if !drifted && !has_skipped {
        return Ok(());
    }

    println!();
    let question = if drifted {
        "Apply these changes?"
    } else {
        "No drift found; apply the uninspected sections anyway?"
    };
    if !confirm(question)? {
        return Ok(());
    }

    let applied = sections::run_all_with_prune(
        &selected,
        &checks,
        config.as_ref(),
        &project_root,
        false,
        !opts.no_prune,
    );
    print_report(&applied);

    if applied.any_failed() {
        anyhow::bail!("Some setup steps failed");
    }

    Ok(())
}

fn confirm(message: &str) -> Result<bool> {
    let prompt = format!(
        "  {} {}{}{} ",
        message,
        style("[").bold().cyan(),
        style("Y/n").bold(),
        style("]").bold().cyan(),
    );

    loop {
        print!("{}", prompt);
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let answer = input.trim().to_lowercase();

        match answer.as_str() {
            "" | "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("    {}", style("Please enter y or n").dim()),
        }
    }
}
