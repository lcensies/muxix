//! `muxix exec [--profile <name>] <agent> [args...]`
//!
//! A thin, stateless launcher: it selects an agent config profile, points the
//! agent's config-dir env var at that profile's derived overlay dir, and execs
//! the agent. It never writes — building overlay dirs is `muxix setup`'s job.
//!
//! Profile selection precedence:
//!   1. `--profile <name>` (the global flag)  — `--profile ""` forces base
//!   2. `default_agent_profile:` in config     — only honored here, not by bare agents
//!   3. otherwise base (the untouched real config dir)

use anyhow::{Context, Result, anyhow, bail};
use std::process::Command;

use crate::agent::agent_profiles as ap;
use crate::agent::setup::Agent;
use crate::config::Config;

/// Resolve the effective agent-profile name from the flag + config default.
///
/// `Some("")` (explicit empty `--profile`) means "use base", overriding any
/// configured default; a missing flag falls through to `default_agent_profile`.
fn resolve_profile(cli_profile: Option<&str>, config: &Config) -> Option<String> {
    match cli_profile {
        Some("") => None,
        Some(name) => Some(name.to_string()),
        None => config
            .default_agent_profile
            .clone()
            .filter(|s| !s.is_empty()),
    }
}

pub fn run(agent_cmd: &str, args: &[String], cli_profile: Option<&str>) -> Result<()> {
    // Load config without resolving *muxix* config profiles: the global
    // `--profile` flag here names an *agent* profile, so passing it into config
    // resolution would error on an unknown muxix profile name.
    let config = Config::load_with_options(
        &std::env::current_dir().unwrap_or_default(),
        None,
        None,
        Some(""),
    )
    .map(|(c, _)| c)
    .unwrap_or_default();

    let mut cmd = Command::new(agent_cmd);
    cmd.args(args);

    if let Some(profile) = resolve_profile(cli_profile, &config) {
        apply_profile(&mut cmd, &profile, agent_cmd, &config)?;
    }

    exec(cmd, agent_cmd)
}

/// Point the agent's config-dir env var at the profile's derived overlay dir.
fn apply_profile(
    cmd: &mut Command,
    profile: &str,
    agent_cmd: &str,
    config: &Config,
) -> Result<()> {
    if !config.agent_profiles.contains_key(profile) {
        let declared: Vec<&str> = config.agent_profiles.keys().map(String::as_str).collect();
        let list = if declared.is_empty() {
            "(none declared)".to_string()
        } else {
            declared.join(", ")
        };
        bail!("unknown agent profile '{profile}'; declared: {list}");
    }

    let agent = Agent::from_command(agent_cmd)
        .ok_or_else(|| anyhow!("unknown agent '{agent_cmd}'; cannot profile it"))?;
    let agent_id = agent.profile_id();

    let env_var = ap::config_dir_env(agent_id).ok_or_else(|| {
        anyhow!("agent '{agent_id}' does not support config-dir profiling")
    })?;

    let dest = ap::build_dir(profile, agent_id)?;
    if !dest.exists() {
        bail!(
            "agent profile '{profile}' has not been built for {agent_id}; run `muxix setup` first \
             (expected {})",
            dest.display()
        );
    }

    cmd.env(env_var, &dest);

    // Session history is per-profile, not shared with base. The derived overlay dir is
    // rebuilt (and wiped) by setup, so sessions cannot live there; they go in the
    // persistent data tree beside it and the agent is pointed at it by its own
    // session-dir env var. Agents without such a var keep writing inside their config
    // dir, where the overlay already isolates them.
    if let Some(session_var) = ap::session_dir_env(agent_id) {
        let sessions = ap::session_dir(profile, agent_id)?;
        std::fs::create_dir_all(&sessions)
            .with_context(|| format!("creating profile session dir {}", sessions.display()))?;
        cmd.env(session_var, &sessions);
    }

    Ok(())
}

#[cfg(unix)]
fn exec(mut cmd: Command, agent_cmd: &str) -> Result<()> {
    use std::os::unix::process::CommandExt;
    // Replace this process so signals and the exit code pass straight through.
    Err(cmd.exec()).with_context(|| format!("failed to exec '{agent_cmd}'"))
}

#[cfg(not(unix))]
fn exec(mut cmd: Command, agent_cmd: &str) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("failed to run '{agent_cmd}'"))?;
    std::process::exit(status.code().unwrap_or(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with_default(default: Option<&str>) -> Config {
        Config {
            default_agent_profile: default.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn flag_wins_over_default() {
        let c = cfg_with_default(Some("corp"));
        assert_eq!(resolve_profile(Some("perso"), &c), Some("perso".into()));
    }

    #[test]
    fn empty_flag_forces_base_over_default() {
        let c = cfg_with_default(Some("corp"));
        assert_eq!(resolve_profile(Some(""), &c), None);
    }

    #[test]
    fn missing_flag_falls_back_to_default() {
        let c = cfg_with_default(Some("corp"));
        assert_eq!(resolve_profile(None, &c), Some("corp".into()));
    }

    #[test]
    fn missing_flag_and_no_default_is_base() {
        let c = cfg_with_default(None);
        assert_eq!(resolve_profile(None, &c), None);
    }
}
