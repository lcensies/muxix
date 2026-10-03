//! `workmux agent …` — manage the project's agent definition registry.
//!
//! `list`  — show all resolved agent definitions (inline + file-based).
//! `show`  — show one definition's capabilities and trust summary.

use anyhow::{Context, Result};
use clap::Subcommand;
use console::style;

use crate::agent::definition::AgentDefinition;
use crate::agent::registry::AgentRegistry;
use crate::{config::Config, git};

#[derive(Subcommand, Debug)]
pub enum AgentRegistryCommand {
    /// List all resolved agent definitions from the registry.
    List,
    /// Show a specific agent definition, including its trust summary.
    Show {
        /// Name of the agent definition to show.
        name: String,
    },
}

pub fn run(command: AgentRegistryCommand) -> Result<()> {
    let repo_root =
        git::get_main_worktree_root().context("Failed to locate the repository root")?;
    let config = Config::load(None).context("Failed to load configuration")?;

    let registry = AgentRegistry::load(
        &config.agent_defs,
        &config.agent_registries,
        &repo_root,
    )
    .context("Failed to load agent registry")?;

    match command {
        AgentRegistryCommand::List => {
            let defs = registry.all();
            if defs.is_empty() {
                println!(concat!(
                    "No agent definitions found.\n\n",
                    "Add inline definitions to .workmux.yaml under `agent_defs:`, or\n",
                    "place <name>.yaml files in .workmux/agents/.\n\n",
                    "Example:\n\n",
                    "  agent_defs:\n",
                    "    planner:\n",
                    "      description: \"Read-only planning agent\"\n",
                    "      permission_mode: plan\n",
                    "      prompt_template: \"Think carefully before coding.\""
                ));
            } else {
                println!("{}", style("Agent definitions").bold().cyan());
                for (name, def) in &defs {
                    let desc = def
                        .description
                        .as_deref()
                        .unwrap_or("(no description)");
                    let perm = def
                        .permission_mode
                        .as_deref()
                        .unwrap_or("(inherited)");
                    let has_template = def.prompt_template.is_some()
                        || def.prompt_ref.is_some();
                    let trust = def.trust_summary();
                    let trust_badge = if trust.elevated {
                        format!(" {}", style("⚠ elevated").yellow())
                    } else {
                        String::new()
                    };
                    println!(
                        "  {} — {}  [perm: {}{}{}]",
                        style(name).bold(),
                        desc,
                        perm,
                        if has_template { ", template" } else { "" },
                        trust_badge,
                    );
                }
            }
        }
        AgentRegistryCommand::Show { name } => {
            match registry.get(&name) {
                None => {
                    anyhow::bail!("agent definition '{}' not found in registry", name);
                }
                Some(def) => print_def(&name, def),
            }
        }
    }
    Ok(())
}

fn print_def(name: &str, def: &AgentDefinition) {
    println!("{}", style(format!("Agent: {name}")).bold().cyan());

    if let Some(ref desc) = def.description {
        println!("  Description:     {desc}");
    }
    if let Some(ref agent_type) = def.agent_type {
        println!("  Agent type:      {agent_type}");
    }
    if let Some(ref model) = def.model {
        println!("  Model:           {model}");
    }
    if let Some(ref mode) = def.permission_mode {
        println!("  Permission mode: {mode}");
    }
    if def.prompt_template.is_some() {
        println!(
            "  Prompt template: {} (inline)",
            style("yes").green()
        );
    } else if let Some(ref r) = def.prompt_ref {
        println!("  Prompt ref:      {r}");
    }

    if let Some(ref bootstrap) = def.bootstrap {
        if !bootstrap.plugins.is_empty() {
            println!("  Plugins:         {}", bootstrap.plugins.join(", "));
        }
        if !bootstrap.skills.is_empty() {
            let names: Vec<String> =
                bootstrap.skills.iter().map(|s| s.display()).collect();
            println!("  Skills:          {}", names.join(", "));
        }
        if !bootstrap.prompt_components.is_empty() {
            println!(
                "  Prompt comps:    {}",
                bootstrap.prompt_components.join(", ")
            );
        }
        if !bootstrap.mcp.is_empty() {
            let names: Vec<&str> = bootstrap.mcp.keys().map(|s| s.as_str()).collect();
            println!("  MCP servers:     {}", names.join(", "));
        }
    }

    let trust = def.trust_summary();
    if trust.elevated {
        println!();
        println!("{}", style("⚠  Trust review").yellow().bold());
        println!("  This definition grants elevated capabilities:");
        for item in &trust.items {
            println!("    • {item}");
        }
        println!(
            "  Review before using from an untrusted source. \
             Remote profiles carry tools/permission_mode/bootstrap \
             which are executable, not just text."
        );
    } else if !trust.items.is_empty() {
        println!();
        println!("Capabilities:");
        for item in &trust.items {
            println!("  • {item}");
        }
    }
}
