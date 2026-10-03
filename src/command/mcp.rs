//! `muxix mcp …` — manage the project's MCP (Model Context Protocol) servers.
//!
//! `sync` renders the `mcp:` section of `.muxix.yaml` into a project
//! `.mcp.json` (which `muxix add` then propagates into each worktree).
//! `status` shows the configured servers and harness integration state.

use anyhow::{Context, Result};
use clap::Subcommand;
use console::style;

use crate::agent::setup::{self, Agent, SignalSupport};
use crate::project_state::CapabilityStatus;
use crate::{config::Config, git, mcp};

#[derive(Subcommand, Debug)]
pub enum McpCommand {
    /// Generate/refresh the project `.mcp.json` from the `mcp:` config section.
    Sync,
    /// Show configured MCP servers and harness integration status.
    Status,
}

pub fn run(command: McpCommand) -> Result<()> {
    let repo_root =
        git::get_main_worktree_root().context("Failed to locate the repository root")?;
    let config = Config::load(None).context("Failed to load configuration")?;

    match command {
        McpCommand::Sync => {
            let written = mcp::sync_agent_mcp_configs(&repo_root, &config)?;
            if written.is_empty() {
                println!(
                    "No MCP servers configured. Add an `mcp:` section to .muxix.yaml, e.g.:\n\n  \
                     mcp:\n    socraticode:\n      command: npx\n      args: [\"-y\", \"socraticode\"]"
                );
            } else {
                mcp::record_synced(&repo_root)?;
                for path in &written {
                    println!("Wrote {}", path.display());
                }
                println!(
                    "  Add `{}` to your `.gitignore` if you want them managed by muxix only.",
                    mcp::MCP_JSON_FILENAME
                );
            }
        }
        McpCommand::Status => {
            let status = mcp::harness_status(&repo_root, &config)?;
            print_status(&status);
        }
    }
    Ok(())
}

fn print_status(status: &mcp::HarnessStatus) {
    println!("{}", style("MCP servers").bold().cyan());
    if status.servers.is_empty() {
        println!("  (none configured — add an `mcp:` section to .muxix.yaml)");
    } else {
        for s in &status.servers {
            let state = if s.enabled {
                style("enabled").green()
            } else {
                style("disabled").dim()
            };
            println!("  {} [{}] — {}", style(&s.name).bold(), state, s.command);
        }
    }

    let synced = if status.mcp_synced && status.mcp_json_exists {
        style("synced").green()
    } else {
        style("not synced").yellow()
    };
    println!(
        "\n.mcp.json: {} ({})",
        if status.mcp_json_exists {
            style("present").green()
        } else {
            style("missing").red()
        },
        synced,
    );
    if !status.mcp_json_exists {
        println!("  Run `muxix mcp sync` to generate it.");
    }

    if !status.integrations.is_empty() {
        println!("\n{}", style("Integrations").bold().cyan());
        for i in &status.integrations {
            println!("  {} — {}", style(&i.name).bold(), cap_label(i.status));
        }
    }

    // Per-agent MCP support: which agents muxix writes config + pre-approval
    // for, and which still need an adapter (so the gap is visible when you pick a
    // new agent like pi).
    println!("\n{}", style("Agent MCP support").bold().cyan());
    for (agent, support) in mcp::support_overview() {
        match support {
            Ok(()) => println!("  {} {}", style("✔").green(), agent.name()),
            Err(reason) => println!(
                "  {} {} — {}",
                style("✗").yellow(),
                agent.name(),
                style(reason).dim()
            ),
        }
    }

    // Per-agent pipeline-signal support: whether the agent emits the harness's
    // pane-keyed readiness/turn-done signals natively, or relies on the
    // agent-agnostic echo probe + content fallback.
    println!(
        "\n{}",
        style("Pipeline signals (harness readiness / turn-done)")
            .bold()
            .cyan()
    );
    for agent in Agent::ALL {
        match setup::signal_support(agent) {
            SignalSupport::Native => {
                println!(
                    "  {} {} — {}",
                    style("✔").green(),
                    agent.name(),
                    style("native").dim()
                )
            }
            SignalSupport::FallbackOnly => println!(
                "  {} {} — {}",
                style("~").yellow(),
                agent.name(),
                style("echo probe + content fallback").dim()
            ),
        }
    }
}

fn cap_label(status: CapabilityStatus) -> console::StyledObject<&'static str> {
    match status {
        CapabilityStatus::Done => style("done").green(),
        CapabilityStatus::InProgress => style("in progress").yellow(),
        CapabilityStatus::Absent => style("not set up").dim(),
    }
}
