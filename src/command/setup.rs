use anyhow::Result;
use console::style;
use std::io::{self, IsTerminal, Write};

use crate::agent_setup::{self, StatusCheck};

pub fn run() -> Result<()> {
    if !io::stdin().is_terminal() {
        anyhow::bail!("workmux setup requires an interactive terminal");
    }

    let checks = agent_setup::check_all();

    if checks.is_empty() {
        println!(
            "No agents detected. Install an agent CLI (Claude Code, OpenCode) to get started."
        );
        return Ok(());
    }

    println!();
    let mut any_needed = false;

    for check in &checks {
        let status_str = match &check.status {
            StatusCheck::Installed => format!("{}", style("configured").green()),
            StatusCheck::NotInstalled => {
                any_needed = true;
                format!("{}", style("not configured").yellow())
            }
            StatusCheck::Error(e) => {
                any_needed = true;
                format!("{} ({})", style("error").red(), e)
            }
        };

        println!(
            "  {} {} ({}): {}",
            style("•").dim(),
            check.agent.name(),
            style(check.reason).dim(),
            status_str
        );
    }
    println!();

    if !any_needed {
        println!(
            "{}",
            style("All agents have status tracking configured.").green()
        );
        return Ok(());
    }

    let needs_setup: Vec<_> = checks
        .iter()
        .filter(|c| matches!(c.status, StatusCheck::NotInstalled | StatusCheck::Error(_)))
        .collect();

    agent_setup::print_description("");
    println!();

    if confirm_install()? {
        let mut any_failed = false;
        for check in &needs_setup {
            match agent_setup::install(check.agent) {
                Ok(msg) => println!("  {} {}", style("✓").green(), msg),
                Err(e) => {
                    println!("  {} {}: {}", style("✗").red(), check.agent.name(), e);
                    any_failed = true;
                }
            }
        }
        println!();
        if any_failed {
            anyhow::bail!("Some installations failed");
        }
    } else {
        println!();
    }

    Ok(())
}

fn confirm_install() -> Result<bool> {
    let prompt = format!(
        "  Install status tracking hooks? {}{}{} ",
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
