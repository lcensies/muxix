//! Agent status tracking setup.
//!
//! Detects which agent CLIs the user has, checks if status tracking
//! hooks are installed, and offers to install them. Used by both the
//! `workmux setup` command and the first-run wizard.

pub mod claude;
pub mod copilot;
pub mod opencode;

use anyhow::{Context, Result};
use console::style;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

/// An agent that supports status tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Agent {
    Claude,
    Copilot,
    OpenCode,
}

impl Agent {
    pub fn name(&self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Copilot => "Copilot CLI",
            Agent::OpenCode => "OpenCode",
        }
    }
}

/// Result of verifying an agent's status tracking.
#[derive(Debug)]
pub enum StatusCheck {
    /// Hooks are installed and working.
    Installed,
    /// Hooks are not installed.
    NotInstalled,
    /// Could not determine status (e.g., invalid JSON in settings file).
    Error(String),
}

/// Result of detecting and checking a single agent.
#[derive(Debug)]
pub struct AgentCheck {
    pub agent: Agent,
    pub reason: &'static str,
    pub status: StatusCheck,
}

/// Detect all known agents and check their status tracking.
///
/// Never fails globally -- per-agent errors are captured in `StatusCheck::Error`.
pub fn check_all() -> Vec<AgentCheck> {
    let mut results = Vec::new();

    if let Some(reason) = claude::detect() {
        let status = match claude::check() {
            Ok(s) => s,
            Err(e) => StatusCheck::Error(e.to_string()),
        };
        results.push(AgentCheck {
            agent: Agent::Claude,
            reason,
            status,
        });
    }

    if let Some(reason) = copilot::detect() {
        let status = match copilot::check() {
            Ok(s) => s,
            Err(e) => StatusCheck::Error(e.to_string()),
        };
        results.push(AgentCheck {
            agent: Agent::Copilot,
            reason,
            status,
        });
    }

    if let Some(reason) = opencode::detect() {
        let status = match opencode::check() {
            Ok(s) => s,
            Err(e) => StatusCheck::Error(e.to_string()),
        };
        results.push(AgentCheck {
            agent: Agent::OpenCode,
            reason,
            status,
        });
    }

    results
}

/// Install status tracking for the given agent.
pub fn install(agent: Agent) -> Result<String> {
    match agent {
        Agent::Claude => claude::install(),
        Agent::Copilot => copilot::install(),
        Agent::OpenCode => opencode::install(),
    }
}

// --- State persistence (declined agents) ---

#[derive(Debug, Default, Serialize, Deserialize)]
struct SetupState {
    #[serde(default)]
    declined: BTreeSet<Agent>,
}

fn setup_state_path() -> Result<PathBuf> {
    Ok(crate::state::store::get_state_dir()?.join("workmux/setup.json"))
}

fn load_setup_state() -> SetupState {
    let Ok(path) = setup_state_path() else {
        return SetupState::default();
    };
    if !path.exists() {
        return SetupState::default();
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

fn save_setup_state(state: &SetupState) -> Result<()> {
    let path = setup_state_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("Failed to create state directory")?;
    }
    let content = serde_json::to_string_pretty(state)?;
    fs::write(&path, content + "\n")?;
    Ok(())
}

pub fn is_declined(agent: Agent) -> bool {
    load_setup_state().declined.contains(&agent)
}

fn mark_declined(agents: &[Agent]) -> Result<()> {
    let mut state = load_setup_state();
    for agent in agents {
        state.declined.insert(*agent);
    }
    save_setup_state(&state)
}

// --- Shared prompt UI ---

/// Print the status tracking description with a mock tmux status bar.
/// `prefix` is printed before each line (e.g. "│ " for the wizard, "" for the command).
pub(crate) fn print_description(prefix: &str) {
    println!("{prefix}  Status tracking shows agent activity in your tmux window list:");
    println!("{prefix}");
    println!(
        "{prefix}    {}  2:user-auth 🤖  3:refactor 💬  {}",
        style("1:main*").reverse(),
        style("4:dark-mode ✅").dim(),
    );
    println!("{prefix}");
    println!("{prefix}  🤖 = working  💬 = waiting for input  ✅ = done");
    println!(
        "{prefix}  {}",
        style("https://workmux.raine.dev/guide/status-tracking").dim()
    );
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

fn print_install_result(agent: Agent, result: &Result<String>) {
    match result {
        Ok(msg) => println!("  {} {}", style("✔").green(), msg),
        Err(e) => println!("  {} {}: {}", style("✗").red(), agent.name(), e),
    }
}

fn install_agents(agents: &[&AgentCheck]) {
    for check in agents {
        let result = install(check.agent);
        print_install_result(check.agent, &result);
    }
}

// --- First-run wizard ---

/// Run the first-run wizard status tracking check.
///
/// Only prompts for detected agents that are NOT installed and NOT
/// previously declined. Designed to be called after the nerdfont wizard.
pub fn prompt_wizard() -> Result<()> {
    if !io::stdin().is_terminal() {
        return Ok(());
    }

    if std::env::var("CI").is_ok() || std::env::var("WORKMUX_TEST").is_ok() {
        return Ok(());
    }

    let checks = check_all();
    let needs_setup: Vec<_> = checks
        .iter()
        .filter(|c| matches!(c.status, StatusCheck::NotInstalled))
        .filter(|c| !is_declined(c.agent))
        .collect();

    if needs_setup.is_empty() {
        return Ok(());
    }

    let dim = style("│").dim();
    let corner_top = style("┌").dim();

    println!();
    println!("{} {}", corner_top, style("Status Tracking").bold().cyan());
    println!("{}", dim);

    for check in &needs_setup {
        println!(
            "{}  Detected {} ({})",
            dim,
            style(check.agent.name()).bold(),
            check.reason
        );
    }

    println!("{}", dim);
    let dim_str = format!("{}", dim);
    print_description(&dim_str);
    println!("{}", dim);

    if confirm_install()? {
        install_agents(&needs_setup);
    } else {
        let agents: Vec<_> = needs_setup.iter().map(|c| c.agent).collect();
        if let Err(e) = mark_declined(&agents) {
            tracing::debug!(?e, "failed to save declined state");
        }
    }

    println!();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_name() {
        assert_eq!(Agent::Claude.name(), "Claude Code");
        assert_eq!(Agent::Copilot.name(), "Copilot CLI");
        assert_eq!(Agent::OpenCode.name(), "OpenCode");
    }

    #[test]
    fn test_agent_serialization() {
        assert_eq!(serde_json::to_string(&Agent::Claude).unwrap(), "\"claude\"");
        assert_eq!(
            serde_json::to_string(&Agent::Copilot).unwrap(),
            "\"copilot\""
        );
        assert_eq!(
            serde_json::to_string(&Agent::OpenCode).unwrap(),
            "\"opencode\""
        );
    }

    #[test]
    fn test_agent_deserialization() {
        let agent: Agent = serde_json::from_str("\"claude\"").unwrap();
        assert_eq!(agent, Agent::Claude);
        let agent: Agent = serde_json::from_str("\"copilot\"").unwrap();
        assert_eq!(agent, Agent::Copilot);
        let agent: Agent = serde_json::from_str("\"opencode\"").unwrap();
        assert_eq!(agent, Agent::OpenCode);
    }

    #[test]
    fn test_setup_state_default_is_empty() {
        let state = SetupState::default();
        assert!(state.declined.is_empty());
    }

    #[test]
    fn test_setup_state_serialization_round_trip() {
        let mut state = SetupState::default();
        state.declined.insert(Agent::Claude);

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: SetupState = serde_json::from_str(&json).unwrap();
        assert!(deserialized.declined.contains(&Agent::Claude));
        assert!(!deserialized.declined.contains(&Agent::OpenCode));
    }

    #[test]
    fn test_setup_state_round_trip_both_agents() {
        let mut state = SetupState::default();
        state.declined.insert(Agent::Claude);
        state.declined.insert(Agent::OpenCode);

        let json = serde_json::to_string_pretty(&state).unwrap();
        let deserialized: SetupState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.declined.len(), 2);
        assert!(deserialized.declined.contains(&Agent::Claude));
        assert!(deserialized.declined.contains(&Agent::OpenCode));
    }

    #[test]
    fn test_setup_state_deserialize_empty_json() {
        let deserialized: SetupState = serde_json::from_str("{}").unwrap();
        assert!(deserialized.declined.is_empty());
    }
}
