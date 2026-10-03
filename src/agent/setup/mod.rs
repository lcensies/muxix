//! Agent status tracking setup.
//!
//! Detects which agent CLIs the user has, checks if status tracking
//! hooks are installed, and offers to install them. Used by both the
//! `muxix setup` command and the first-run wizard.

pub mod claude;
pub mod codex;
pub mod copilot;
pub mod gemini;
pub mod omp;
pub mod opencode;
pub mod pi;
pub mod spec;

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
    Codex,
    Copilot,
    Gemini,
    OpenCode,
    Pi,
    Omp,
}

impl Agent {
    /// Every known coding agent, in display order. Keep exhaustive: code that
    /// must handle each agent (e.g. MCP support) iterates this, so a new variant
    /// surfaces everywhere it needs a decision.
    pub const ALL: [Agent; 7] = [
        Agent::Claude,
        Agent::Codex,
        Agent::Copilot,
        Agent::Gemini,
        Agent::OpenCode,
        Agent::Pi,
        Agent::Omp,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Codex => "Codex",
            Agent::Copilot => "Copilot CLI",
            Agent::Gemini => "Gemini CLI",
            Agent::OpenCode => "OpenCode",
            Agent::Pi => "pi",
            Agent::Omp => "omp",
        }
    }

    /// Canonical lowercase id — the single source of truth that bridges the three
    /// agent identity axes: it equals the agent's command stem, its
    /// [`crate::agent::profile::AgentProfile::name`], and its
    /// [`crate::agent::identity::AgentKind::as_str`]. (Consistency is enforced by
    /// tests so the three can never drift.)
    pub fn profile_id(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Codex => "codex",
            Agent::Copilot => "copilot",
            Agent::Gemini => "gemini",
            Agent::OpenCode => "opencode",
            Agent::Pi => "pi",
            Agent::Omp => "omp",
        }
    }

    /// Inverse of [`Agent::profile_id`]. `None` for ids that are not (yet) a
    /// first-class `Agent` (e.g. `kiro-cli`, `vibe`, `kimi`, `default`).
    pub fn from_profile_id(id: &str) -> Option<Agent> {
        Agent::ALL.into_iter().find(|a| a.profile_id() == id)
    }

    /// Whether a config key refers to this agent. Accepts both the short
    /// [`Agent::profile_id`] (e.g. `pi`, `claude`) and the display
    /// [`Agent::name`] lowercased (e.g. `claude code`), so config sections can
    /// use whichever reads best. Matching is case- and whitespace-insensitive.
    pub fn matches_config_key(self, key: &str) -> bool {
        let k = key.trim().to_lowercase();
        k == self.profile_id() || k == self.name().to_lowercase()
    }

    /// Bridge the **command-string** axis (what the user configures / runs) to the
    /// `Agent` enum. Uses the profile resolver (handles `env`/`VAR=` wrappers and
    /// symlink stems), with a direct stem fallback for agents that have no
    /// `AgentProfile` yet (e.g. Copilot). `None` for unknown commands.
    pub fn from_command(cmd: &str) -> Option<Agent> {
        let profile = crate::agent::profile::resolve_profile_for_display(Some(cmd)).name();
        if let Some(a) = Agent::from_profile_id(profile) {
            return Some(a);
        }
        // Fallback: bare stem of the last whitespace-delimited token.
        let token = cmd.split_whitespace().next_back().unwrap_or(cmd);
        let stem = std::path::Path::new(token)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(token);
        Agent::from_profile_id(stem)
    }
}

/// The format an agent's settings file is written in, which decides how a
/// declared `settings:` merge patch is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsFormat {
    Json,
    Yaml,
}

/// Where this agent keeps its own settings file — the file the agent itself
/// reads, not a muxix-owned copy — and in which format.
///
/// `None` for an agent whose settings muxix cannot patch: Codex keeps its
/// config in TOML (`~/.codex/config.toml`), which RFC 7386 merge-patch
/// semantics cannot express. A declared settings patch for those is skipped
/// rather than written to a guessed path or an unexpressible format.
pub fn settings_target(agent: Agent) -> Option<(PathBuf, SettingsFormat)> {
    let path = match agent {
        Agent::Pi => pi::settings_file(),
        Agent::Omp => omp::settings_file(),
        Agent::Claude => claude::settings_file(),
        Agent::Gemini => gemini::settings_file(),
        Agent::OpenCode => opencode::settings_file(),
        Agent::Copilot => copilot::settings_file(),
        Agent::Codex => None,
    }?;
    // omp's store is YAML (`config.yml`); every other supported agent's is JSON.
    let format = match agent {
        Agent::Omp => SettingsFormat::Yaml,
        _ => SettingsFormat::Json,
    };
    Some((path, format))
}

/// The agent's settings file path, ignoring its format.
#[allow(dead_code)]
pub fn settings_file(agent: Agent) -> Option<PathBuf> {
    settings_target(agent).map(|(path, _)| path)
}

/// Result of verifying an agent's status tracking.
#[derive(Debug)]
pub enum StatusCheck {
    /// Hooks are installed and current (all required commands present).
    Installed,
    /// Some muxix hooks are present but the required set is incomplete —
    /// typically muxix was updated (new hooks added to plugin.json) without a
    /// re-setup. `missing` lists the required hook commands not found. Re-running
    /// install merges them in.
    Stale { missing: Vec<String> },
    /// Hooks are not installed.
    NotInstalled,
    /// Could not determine status (e.g., invalid JSON in settings file).
    Error(String),
}

impl StatusCheck {
    /// Whether install/re-setup should run (not installed, or installed but stale).
    pub fn needs_setup(&self) -> bool {
        matches!(self, StatusCheck::NotInstalled | StatusCheck::Stale { .. })
    }
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

    if let Some(reason) = codex::detect() {
        let status = match codex::check() {
            Ok(s) => s,
            Err(e) => StatusCheck::Error(e.to_string()),
        };
        results.push(AgentCheck {
            agent: Agent::Codex,
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

    if let Some(reason) = gemini::detect() {
        let status = match gemini::check() {
            Ok(s) => s,
            Err(e) => StatusCheck::Error(e.to_string()),
        };
        results.push(AgentCheck {
            agent: Agent::Gemini,
            reason,
            status,
        });
    }

    if let Some(reason) = pi::detect() {
        let status = match pi::check() {
            Ok(s) => s,
            Err(e) => StatusCheck::Error(e.to_string()),
        };
        results.push(AgentCheck {
            agent: Agent::Pi,
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

    if let Some(reason) = omp::detect() {
        let status = match omp::check() {
            Ok(s) => s,
            Err(e) => StatusCheck::Error(e.to_string()),
        };
        results.push(AgentCheck {
            agent: Agent::Omp,
            reason,
            status,
        });
    }

    results
}

/// Whether the given agent's CLI is detected on this machine.
///
/// Thin dispatch over each agent module's `detect()` (which returns a
/// human-readable reason when present). Used to decide which agents'
/// native config files are worth materializing.
pub fn is_detected(agent: Agent) -> bool {
    match agent {
        Agent::Claude => claude::detect().is_some(),
        Agent::Codex => codex::detect().is_some(),
        Agent::Copilot => copilot::detect().is_some(),
        Agent::Gemini => gemini::detect().is_some(),
        Agent::OpenCode => opencode::detect().is_some(),
        Agent::Pi => pi::detect().is_some(),
        Agent::Omp => omp::detect().is_some(),
    }
}

/// Install status tracking for the given agent.
pub fn install(agent: Agent) -> Result<String> {
    match agent {
        Agent::Claude => claude::install(),
        Agent::Codex => codex::install(),
        Agent::Copilot => copilot::install(),
        Agent::Gemini => gemini::install(),
        Agent::OpenCode => opencode::install(),
        Agent::Pi => pi::install(),
        Agent::Omp => omp::install(),
    }
}

/// Whether an agent emits muxix's **pane-keyed pipeline signals**
/// (`session-ready`, `turn-done`) natively via its installed hook/plugin, or the
/// harness must rely on the agent-agnostic echo probe + content idle fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalSupport {
    /// Emits `session-ready`/`turn-done` itself → deterministic readiness gate
    /// and turn completion.
    Native,
    /// No native signals → runner uses the echo probe for readiness and the
    /// content idle fallback for turn completion. Still functional, less precise.
    FallbackOnly,
}

/// Pipeline-signal support for an agent. **Exhaustive** over [`Agent`], so adding
/// a new agent (or wiring its hook/plugin) forces a decision here and shows up in
/// `muxix mcp status`.
pub fn signal_support(agent: Agent) -> SignalSupport {
    match agent {
        // Claude: SessionStart/Stop hooks in .claude-plugin/plugin.json.
        Agent::Claude => SignalSupport::Native,
        // OpenCode: resources/opencode/plugins/muxix-status.ts (init + idle).
        Agent::OpenCode => SignalSupport::Native,
        // pi: .pi/extensions/muxix-status.ts (load + agent_end).
        Agent::Pi => SignalSupport::Native,
        // omp (oh-my-pi): pi-compatible extension (load + agent_end).
        Agent::Omp => SignalSupport::Native,
        // No signal emission wired into these agents' configs yet.
        Agent::Gemini | Agent::Codex | Agent::Copilot => SignalSupport::FallbackOnly,
    }
}

/// Trait for agent-specific bootstrap. Default `apply_prompt` writes inline sentinels
/// to the path returned by `instructions_path`. Agents with different mechanisms
/// (e.g. Claude's @-reference) override `apply_prompt` directly.
pub trait AgentBootstrapper {
    /// Path to the agent's global instructions file.
    fn instructions_path(&self) -> PathBuf;

    /// Apply bootstrap prompt. Default: inline sentinels in `instructions_path()`.
    fn apply_prompt(&self, prompt: &str) -> Result<()> {
        inline_sentinels(&self.instructions_path(), prompt)
    }

    /// The prompt currently applied, read back from `instructions_path()`:
    /// the sentinel block when present, else the whole file (raw-prompt
    /// agents). `None` when nothing is installed. Lets `bootstrap` stay quiet
    /// on a converged machine instead of reporting "updated" every run.
    fn current_prompt(&self) -> Option<String> {
        let text = fs::read_to_string(self.instructions_path()).ok()?;
        let block = match (text.find(SENTINEL_BEGIN), text.find(SENTINEL_END)) {
            (Some(s), Some(e)) if e > s => &text[s + SENTINEL_BEGIN.len()..e],
            _ => text.as_str(),
        };
        Some(block.trim().to_string())
    }
}

const SENTINEL_BEGIN: &str = "<!-- muxix-bootstrap-begin -->";
const SENTINEL_END: &str = "<!-- muxix-bootstrap-end -->";

/// Write prompt content into a file using sentinel comments (idempotent).
pub(super) fn inline_sentinels(path: &std::path::Path, prompt: &str) -> Result<()> {
    let existing = if path.exists() {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    fs::write(path, splice_sentinels(&existing, prompt))?;
    Ok(())
}

/// Replace (or append) the muxix-managed sentinel region in `existing`.
///
/// Pure so the agent-profile overlay can generate the same body for a file it
/// shares with the user without writing to the base file.
pub fn splice_sentinels(existing: &str, prompt: &str) -> String {
    const BEGIN: &str = SENTINEL_BEGIN;
    const END: &str = SENTINEL_END;

    let new_block = format!("{}\n{}\n{}", BEGIN, prompt.trim(), END);
    if let (Some(start), Some(end)) = (existing.find(BEGIN), existing.find(END)) {
        let end_pos = end + END.len();
        format!(
            "{}{}{}",
            &existing[..start],
            new_block,
            &existing[end_pos..]
        )
    } else if existing.is_empty() {
        format!("{}\n", new_block)
    } else {
        format!("{}\n\n{}\n", existing.trim_end(), new_block)
    }
}

/// Set a (possibly nested) string key in a JSON settings file, creating the
/// file and any intermediate objects. Every other key is preserved.
///
/// Returns `false` when the key already held this value, so callers can stay
/// quiet on re-runs.
pub(super) fn set_json_string(path: &std::path::Path, keys: &[&str], value: &str) -> Result<bool> {
    let (last, parents) = keys
        .split_last()
        .ok_or_else(|| anyhow::anyhow!("set_json_string needs at least one key"))?;

    let mut root: serde_json::Value = if path.exists() {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("{} is not valid JSON", path.display()))?
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let mut cursor = &mut root;
    for key in parents {
        cursor = cursor
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("{}: `{key}` is not an object", path.display()))?
            .entry((*key).to_string())
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    }
    let obj = cursor
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("{}: `{last}` is not inside an object", path.display()))?;

    if obj.get(*last).and_then(|v| v.as_str()) == Some(value) {
        return Ok(false);
    }
    obj.insert((*last).to_string(), value.into());

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }
    let output = serde_json::to_string_pretty(&root)?;
    fs::write(path, output + "\n")
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(true)
}

/// Apply bootstrap prompt to the given agent. Returns Ok(false) if not supported.
pub fn bootstrap(
    agent: Agent,
    prompt: &str,
    config: Option<&crate::bootstrap::BootstrapConfig>,
) -> Result<BootstrapOutcome> {
    let b: Option<Box<dyn AgentBootstrapper>> = match agent {
        Agent::Claude => claude::Bootstrapper::new().map(|b| Box::new(b) as _),
        Agent::OpenCode => opencode::Bootstrapper::new().map(|b| Box::new(b) as _),
        Agent::Gemini => gemini::Bootstrapper::new().map(|b| Box::new(b) as _),
        Agent::Pi => {
            let method = config
                .and_then(|c| c.pi.as_ref())
                .map(|c| c.injection_method.clone())
                .unwrap_or_default();
            pi::Bootstrapper::new_with_method(method).map(|b| Box::new(b) as _)
        }
        Agent::Omp => {
            let method = config
                .and_then(|c| c.pi.as_ref())
                .map(|c| c.injection_method.clone())
                .unwrap_or_default();
            omp::Bootstrapper::new_with_method(method).map(|b| Box::new(b) as _)
        }
        // Codex and Copilot have no plugin installer, so their instructions
        // file is the ONLY channel a declared prompt component (or a feature's
        // prompt fallback) can reach them through.
        Agent::Codex => codex::Bootstrapper::new().map(|b| Box::new(b) as _),
        Agent::Copilot => copilot::Bootstrapper::new().map(|b| Box::new(b) as _),
    };
    match b {
        Some(b) => {
            if b.current_prompt().as_deref() == Some(prompt.trim()) {
                return Ok(BootstrapOutcome::UpToDate);
            }
            b.apply_prompt(prompt).map(|_| BootstrapOutcome::Updated)
        }
        None => Ok(BootstrapOutcome::Unsupported),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapOutcome {
    Updated,
    UpToDate,
    /// The agent has no bootstrap mechanism at all.
    Unsupported,
}

// --- State persistence (declined agents) ---

#[derive(Debug, Default, Serialize, Deserialize)]
struct SetupState {
    #[serde(default)]
    declined: BTreeSet<Agent>,
    #[serde(default)]
    declined_skills: BTreeSet<Agent>,
}

fn setup_state_path() -> Result<PathBuf> {
    Ok(crate::state::store::get_state_dir()?.join("setup.json"))
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

fn mark_skills_declined(agents: &[Agent]) -> Result<()> {
    let mut state = load_setup_state();
    for agent in agents {
        state.declined_skills.insert(*agent);
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
        style("https://github.com/lcensies/muxix/blob/main/docs/guide/status-tracking.md").dim()
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

    if std::env::var("CI").is_ok() || std::env::var("MUXIX_TEST").is_ok() {
        return Ok(());
    }

    let checks = check_all();
    let needs_hooks: Vec<_> = checks
        .iter()
        .filter(|c| c.status.needs_setup())
        .filter(|c| !is_declined(c.agent))
        .collect();

    if needs_hooks.is_empty() {
        return Ok(());
    }

    let dim = style("│").dim();
    let corner_top = style("┌").dim();

    // Status tracking hooks
    if !needs_hooks.is_empty() {
        println!();
        println!("{} {}", corner_top, style("Status Tracking").bold().cyan());
        println!("{}", dim);

        for check in &needs_hooks {
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
            install_agents(&needs_hooks);
        } else {
            let agents: Vec<_> = needs_hooks.iter().map(|c| c.agent).collect();
            if let Err(e) = mark_declined(&agents) {
                tracing::debug!(?e, "failed to save declined state");
            }
        }
    }

    // Skill installation (only during first-run wizard, not for existing users)
    {
        let skill_agents: Vec<Agent> = checks
            .iter()
            .map(|c| c.agent)
            .filter(|a| crate::skills::needs_install(*a))
            .collect();

        if !skill_agents.is_empty() {
            println!("{}", dim);
            println!("{} {}", dim, style("Skills").bold().cyan());
            println!("{}", dim);

            let skill_names: Vec<_> = crate::skills::BUNDLED_SKILLS
                .iter()
                .map(|s| s.name)
                .collect();
            println!("{}  muxix includes skills: {}", dim, skill_names.join(", "));
            println!(
                "{}  Learn more: {}",
                dim,
                style("https://github.com/lcensies/muxix/blob/main/docs/guide/skills.md").dim()
            );
            println!("{}", dim);

            if confirm_install_skills()? {
                for agent in &skill_agents {
                    match crate::skills::install_skills(*agent) {
                        Ok(msg) => println!("  {}", msg),
                        Err(e) => println!("  {} {}: {}", style("✗").red(), agent.name(), e),
                    }
                }
            } else if let Err(e) = mark_skills_declined(&skill_agents) {
                tracing::debug!(?e, "failed to save declined skills state");
            }
        }
    }

    println!();
    Ok(())
}

fn confirm_install_skills() -> Result<bool> {
    let prompt = format!(
        "  Install skills? {}{}{} ",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every agent must have an explicit, documented decision for every harness
    /// capability. This is the guard against a new `Agent` variant (or a stale
    /// `_ => None` arm) quietly dropping a declared item; it states the same
    /// support set as the table in `docs/guide/bootstrap.md`.
    #[test]
    fn harness_capability_coverage_is_declared_for_every_agent() {
        use crate::bootstrap::{subagents_dir, subagents_unsupported_reason};
        use crate::mcp::targets::{McpSupport, mcp_support};

        for agent in Agent::ALL {
            // Skills and MCP: every agent, no exceptions.
            assert!(
                crate::skills::skills_dir(agent).is_some(),
                "{}: no skills dir",
                agent.name()
            );
            assert!(
                matches!(mcp_support(agent), McpSupport::Supported(_)),
                "{}: no MCP target",
                agent.name()
            );

            // Subagents and settings may be unsupported, but only with a reason.
            if subagents_dir(agent).is_none() {
                assert!(
                    subagents_unsupported_reason(agent).is_some(),
                    "{}: no subagents dir and no reason",
                    agent.name()
                );
            }
            if settings_target(agent).is_none() {
                assert_eq!(
                    agent,
                    Agent::Codex,
                    "{}: settings gap undocumented",
                    agent.name()
                );
            }

            // Profiling: profilable unless the agent has no config-dir redirect
            // upstream (gemini, opencode).
            let profilable =
                crate::agent::agent_profiles::config_dir_env(agent.profile_id()).is_some();
            assert_eq!(
                profilable,
                !matches!(agent, Agent::Gemini | Agent::OpenCode),
                "{}: profiling decision changed",
                agent.name()
            );
        }
    }

    #[test]
    fn settings_target_declares_a_format_per_agent() {
        for agent in Agent::ALL {
            match agent {
                // TOML: RFC 7386 merge-patch semantics do not map onto it.
                Agent::Codex => assert!(settings_target(agent).is_none()),
                _ => {
                    let (path, format) =
                        settings_target(agent).unwrap_or_else(|| panic!("{}", agent.name()));
                    match format {
                        SettingsFormat::Yaml => assert_eq!(
                            path.extension().and_then(|e| e.to_str()),
                            Some("yml"),
                            "{path:?}"
                        ),
                        SettingsFormat::Json => assert_eq!(
                            path.extension().and_then(|e| e.to_str()),
                            Some("json"),
                            "{path:?}"
                        ),
                    }
                }
            }
        }
    }

    #[test]
    fn omp_settings_are_yaml_and_copilot_is_json() {
        // omp keeps settings in config.yml and never reads pi's settings.json.
        let (omp_path, omp_format) = settings_target(Agent::Omp).unwrap();
        assert!(omp_path.ends_with("config.yml"), "{omp_path:?}");
        assert_eq!(omp_format, SettingsFormat::Yaml);

        let (copilot_path, copilot_format) = settings_target(Agent::Copilot).unwrap();
        assert!(copilot_path.ends_with("settings.json"), "{copilot_path:?}");
        assert_eq!(copilot_format, SettingsFormat::Json);
    }

    #[test]
    fn test_agent_name() {
        assert_eq!(Agent::Claude.name(), "Claude Code");
        assert_eq!(Agent::Codex.name(), "Codex");
        assert_eq!(Agent::Copilot.name(), "Copilot CLI");
        assert_eq!(Agent::Gemini.name(), "Gemini CLI");
        assert_eq!(Agent::OpenCode.name(), "OpenCode");
        assert_eq!(Agent::Pi.name(), "pi");
    }

    #[test]
    fn signal_support_priority_agents_are_native() {
        // Claude/OpenCode/pi emit pipeline signals via their hook/plugin; the rest
        // fall back to the echo probe + content idle detection.
        assert_eq!(signal_support(Agent::Claude), SignalSupport::Native);
        assert_eq!(signal_support(Agent::OpenCode), SignalSupport::Native);
        assert_eq!(signal_support(Agent::Pi), SignalSupport::Native);
        assert_eq!(signal_support(Agent::Gemini), SignalSupport::FallbackOnly);
        assert_eq!(signal_support(Agent::Codex), SignalSupport::FallbackOnly);
        assert_eq!(signal_support(Agent::Copilot), SignalSupport::FallbackOnly);
    }

    #[test]
    fn test_agent_serialization() {
        assert_eq!(serde_json::to_string(&Agent::Claude).unwrap(), "\"claude\"");
        assert_eq!(serde_json::to_string(&Agent::Codex).unwrap(), "\"codex\"");
        assert_eq!(
            serde_json::to_string(&Agent::Copilot).unwrap(),
            "\"copilot\""
        );
        assert_eq!(serde_json::to_string(&Agent::Gemini).unwrap(), "\"gemini\"");
        assert_eq!(
            serde_json::to_string(&Agent::OpenCode).unwrap(),
            "\"opencode\""
        );
        assert_eq!(serde_json::to_string(&Agent::Pi).unwrap(), "\"pi\"");
    }

    #[test]
    fn test_agent_deserialization() {
        let agent: Agent = serde_json::from_str("\"claude\"").unwrap();
        assert_eq!(agent, Agent::Claude);
        let agent: Agent = serde_json::from_str("\"codex\"").unwrap();
        assert_eq!(agent, Agent::Codex);
        let agent: Agent = serde_json::from_str("\"copilot\"").unwrap();
        assert_eq!(agent, Agent::Copilot);
        let agent: Agent = serde_json::from_str("\"gemini\"").unwrap();
        assert_eq!(agent, Agent::Gemini);
        let agent: Agent = serde_json::from_str("\"opencode\"").unwrap();
        assert_eq!(agent, Agent::OpenCode);
        let agent: Agent = serde_json::from_str("\"pi\"").unwrap();
        assert_eq!(agent, Agent::Pi);
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
    fn test_setup_state_round_trip_multiple_agents() {
        let mut state = SetupState::default();
        state.declined.insert(Agent::Claude);
        state.declined.insert(Agent::Codex);
        state.declined.insert(Agent::OpenCode);
        state.declined.insert(Agent::Pi);

        let json = serde_json::to_string_pretty(&state).unwrap();
        let deserialized: SetupState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.declined.len(), 4);
        assert!(deserialized.declined.contains(&Agent::Claude));
        assert!(deserialized.declined.contains(&Agent::Codex));
        assert!(deserialized.declined.contains(&Agent::OpenCode));
    }

    #[test]
    fn test_setup_state_deserialize_empty_json() {
        let deserialized: SetupState = serde_json::from_str("{}").unwrap();
        assert!(deserialized.declined.is_empty());
    }

    #[test]
    fn test_set_json_string() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.json");

        // Creates file and parent dirs.
        assert!(set_json_string(&path, &["theme"], "catppuccin").unwrap());
        // Idempotent: no rewrite when the value already matches.
        assert!(!set_json_string(&path, &["theme"], "catppuccin").unwrap());

        // Nested key creates the intermediate object; siblings survive.
        assert!(set_json_string(&path, &["ui", "theme"], "nord").unwrap());
        let v: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["theme"], "catppuccin");
        assert_eq!(v["ui"]["theme"], "nord");
    }
}
