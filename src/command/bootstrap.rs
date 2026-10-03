//! `muxix bootstrap` — declare what an agent harness should contain.
//!
//! Every entry lives in the `bootstrap:` block of a config file; `muxix setup`
//! installs it and records it in the managed manifest, and removing the
//! declaration is what uninstalls it. Installing through an agent's own
//! installer instead leaves the item undeclared and unmanaged, which is the
//! drift this command exists to prevent.

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

use crate::agent::setup::Agent;
use crate::command::setup::{Section, SetupOptions};
use crate::config::Config;

#[derive(Debug, Args)]
pub struct BootstrapArgs {
    #[command(subcommand)]
    pub command: BootstrapCommand,
}

#[derive(Debug, Subcommand)]
pub enum BootstrapCommand {
    /// Agent plugins/extensions (installed with the agent's own installer)
    Plugin {
        #[command(subcommand)]
        action: Action,
    },
    /// Skills, from a local path or a URL
    Skill {
        #[command(subcommand)]
        action: Action,
    },
    /// Subagent definitions (`.md` files)
    Subagent {
        #[command(subcommand)]
        action: Action,
    },
    /// Prompt components, by name
    Prompt {
        #[command(subcommand)]
        action: Action,
    },
    /// Show what each detected agent resolves to, and where it came from
    List {
        /// Only this agent
        #[arg(long, value_name = "NAME")]
        agent: Option<String>,
    },
    /// Apply the declared config with a full non-interactive `muxix setup`
    Sync,
}

#[derive(Debug, Subcommand)]
pub enum Action {
    /// Declare one or more entries
    Add {
        /// Specs: a plugin spec, skill path/URL, subagent file, or component name
        #[arg(required = true)]
        specs: Vec<String>,
        #[command(flatten)]
        target: Target,
    },
    /// Undeclare one or more entries (setup then removes them)
    #[command(alias = "remove")]
    Rm {
        #[arg(required = true)]
        specs: Vec<String>,
        #[command(flatten)]
        target: Target,
    },
}

#[derive(Debug, Args, Clone)]
pub struct Target {
    /// Declare for this agent only (default: every agent)
    #[arg(long, value_name = "NAME")]
    pub agent: Option<String>,
    /// Write to the global config
    #[arg(long, conflicts_with = "project")]
    pub global: bool,
    /// Write to the project `.muxix.yaml`
    #[arg(long)]
    pub project: bool,
    /// Declare only; do not run `muxix setup`
    #[arg(long = "no-sync")]
    pub no_sync: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Kind {
    Plugin,
    Skill,
    Subagent,
    Prompt,
}

impl Kind {
    /// The shared list key, and the per-agent one under `bootstrap.agents.<a>`.
    fn keys(self) -> (&'static str, &'static str) {
        match self {
            Kind::Plugin => ("default_plugins", "additional_plugins"),
            Kind::Skill => ("default_skills", "additional_skills"),
            Kind::Subagent => ("default_subagents", "additional_subagents"),
            Kind::Prompt => ("default_prompt_components", "additional_prompt_components"),
        }
    }

    /// The setup section that installs this kind.
    fn section(self) -> Section {
        match self {
            Kind::Plugin => Section::Plugins,
            Kind::Skill => Section::Skills,
            Kind::Subagent => Section::Subagents,
            Kind::Prompt => Section::Prompts,
        }
    }
}

/// Resolve an agent name the same way config keys are matched.
fn parse_agent(name: &str) -> Result<Agent> {
    Agent::ALL
        .into_iter()
        .find(|a| a.matches_config_key(name))
        .with_context(|| {
            let known: Vec<&str> = Agent::ALL.iter().map(|a| a.profile_id()).collect();
            format!("Unknown agent '{name}' (known: {})", known.join(", "))
        })
}

/// The config file an edit lands in.
fn target_file(t: &Target) -> Result<PathBuf> {
    let global_path = || {
        crate::config::global_config_path()
            .or_else(|| crate::xdg::config_dir().ok().map(|d| d.join("config.yaml")))
            .context("Could not determine the global config path")
    };
    let file = if t.global {
        global_path()?
    } else {
        let cwd = std::env::current_dir().unwrap_or_default();
        let project = crate::config::find_project_config(&cwd).ok().flatten();
        match (project, t.project) {
            (Some(loc), _) => loc.config_path,
            (None, true) => bail!("No project .muxix.yaml found — run inside a project or use --global"),
            (None, false) => global_path()?,
        }
    };
    // A store symlink (Home Manager / NixOS) is the common read-only case; the
    // edit would otherwise fail deep inside edit_file_at with a bare EACCES.
    if std::fs::metadata(&file).is_ok_and(|m| m.permissions().readonly()) {
        bail!(
            "{} is read-only (managed by Nix/Home Manager?) — declare the entry in its source \
             (e.g. `programs.muxix` in your nix config) and rebuild, or use --project",
            file.display()
        );
    }
    Ok(file)
}

/// Key path into the config for this kind and target.
fn key_path(kind: Kind, agent: Option<Agent>) -> Vec<String> {
    let (shared, per_agent) = kind.keys();
    match agent {
        Some(a) => vec![
            "bootstrap".into(),
            "agents".into(),
            a.profile_id().into(),
            per_agent.into(),
        ],
        None => vec!["bootstrap".into(), shared.into()],
    }
}

/// The list currently declared at `path` in this file, as raw YAML values.
fn read_list(file: &Path, path: &[String]) -> Result<Vec<serde_yaml::Value>> {
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("Failed to read {}", file.display())),
    };
    let mut value: serde_yaml::Value = serde_yaml::from_str(&text)
        .with_context(|| format!("Failed to parse {}", file.display()))?;
    for key in path {
        value = match value.get(key.as_str()) {
            Some(v) => v.clone(),
            None => return Ok(Vec::new()),
        };
    }
    Ok(value.as_sequence().cloned().unwrap_or_default())
}

/// Whether a declared entry names `spec`. Entries are either a bare string or a
/// map with `path`/`url`, so both forms have to be checked.
fn entry_matches(entry: &serde_yaml::Value, spec: &str) -> bool {
    match entry {
        serde_yaml::Value::String(s) => s == spec,
        serde_yaml::Value::Mapping(_) => ["path", "url"]
            .iter()
            .any(|k| entry.get(*k).and_then(|v| v.as_str()) == Some(spec)),
        _ => false,
    }
}

/// Render a list back as a YAML block, or `None` when it is empty (which
/// removes the key rather than leaving `key: []` behind).
fn render(key: &str, list: &[serde_yaml::Value]) -> Result<Option<String>> {
    if list.is_empty() {
        return Ok(None);
    }
    let body = serde_yaml::to_string(&list)?;
    Ok(Some(format!("{key}:\n{}", body.trim_end())))
}

/// Drop parents an edit left empty, so removing the last per-agent entry does
/// not leave `agents: {pi: null}` behind — which is not a valid override map
/// and would fail the next config load.
fn prune_empty_parents(file: &Path, path: &[String]) -> Result<()> {
    for depth in (1..path.len()).rev() {
        let prefix = &path[..depth];
        let text = std::fs::read_to_string(file)?;
        let mut value: serde_yaml::Value = serde_yaml::from_str(&text)?;
        let mut found = true;
        for key in prefix {
            value = match value.get(key.as_str()) {
                Some(v) => v.clone(),
                None => {
                    found = false;
                    break;
                }
            };
        }
        let empty = found
            && match &value {
                serde_yaml::Value::Null => true,
                serde_yaml::Value::Mapping(m) => m.is_empty(),
                _ => false,
            };
        if !empty {
            break;
        }
        let refs: Vec<&str> = prefix.iter().map(String::as_str).collect();
        crate::config::edit::edit_file_at(file, &refs, None)?;
    }
    Ok(())
}

fn run_action(kind: Kind, action: Action) -> Result<()> {
    let (specs, target, adding) = match action {
        Action::Add { specs, target } => (specs, target, true),
        Action::Rm { specs, target } => (specs, target, false),
    };

    let agent = target.agent.as_deref().map(parse_agent).transpose()?;
    let file = target_file(&target)?;
    let path = key_path(kind, agent);
    let key = path.last().expect("key path is never empty").clone();

    let mut list = read_list(&file, &path)?;
    let mut changed = Vec::new();
    for spec in &specs {
        let present = list.iter().any(|e| entry_matches(e, spec));
        match (adding, present) {
            (true, true) => println!("already declared: {spec}"),
            (true, false) => {
                list.push(serde_yaml::Value::String(spec.clone()));
                changed.push(spec.clone());
            }
            (false, true) => {
                list.retain(|e| !entry_matches(e, spec));
                changed.push(spec.clone());
            }
            (false, false) => bail!(
                "{spec} is not declared in {}.{key} ({})",
                path[..path.len() - 1].join("."),
                file.display()
            ),
        }
    }

    if changed.is_empty() {
        return Ok(());
    }

    let refs: Vec<&str> = path.iter().map(String::as_str).collect();
    crate::config::edit::edit_file_at(&file, &refs, render(&key, &list)?.as_deref())?;
    prune_empty_parents(&file, &path)?;

    let verb = if adding { "declared" } else { "undeclared" };
    let scope = match agent {
        Some(a) => format!("for {}", a.profile_id()),
        None => "for every agent".to_string(),
    };
    println!(
        "{verb} {} {scope} in {}",
        changed.join(", "),
        file.display()
    );

    if target.no_sync {
        println!("not applied — run `muxix bootstrap sync` (or `muxix setup`) to apply");
        return Ok(());
    }
    sync(vec![kind.section()])
}

/// Apply the declared config. Reported but not fatal: the declaration stands
/// either way, and a transient install failure should not be mistaken for a
/// config problem.
fn sync(only: Vec<Section>) -> Result<()> {
    let opts = SetupOptions {
        non_interactive: true,
        only,
        profile: crate::config::profiles::cli_profile().map(str::to_string),
        ..Default::default()
    };
    match crate::command::setup::run_automated(&opts) {
        Ok(0) => Ok(()),
        Ok(code) => {
            eprintln!("muxix setup exited with {code}; the declaration stands");
            Ok(())
        }
        Err(e) => {
            eprintln!("muxix setup failed: {e:#}; the declaration stands");
            Ok(())
        }
    }
}

fn run_list(agent_filter: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let config = Config::load_from(&cwd, None).unwrap_or_default();
    let Some(bootstrap) = config.bootstrap.as_ref() else {
        println!("no bootstrap config");
        return Ok(());
    };

    let wanted = agent_filter.as_deref().map(parse_agent).transpose()?;
    let agents: Vec<Agent> = match wanted {
        Some(a) => vec![a],
        None => Agent::ALL
            .into_iter()
            .filter(|a| crate::agent::setup::is_detected(*a))
            .collect(),
    };
    if agents.is_empty() {
        println!("no agents detected");
        return Ok(());
    }

    for agent in agents {
        let plugins = bootstrap.plugins_for(agent);
        let skills = bootstrap.skill_entries_for(agent);
        let subagents = bootstrap.subagents_for(agent);
        let prompts = bootstrap.prompt_components_for(agent);
        let settings = bootstrap.settings_for(agent);
        if plugins.is_empty()
            && skills.is_empty()
            && subagents.is_empty()
            && prompts.is_empty()
            && settings.is_none()
        {
            continue;
        }
        println!("{}", agent.profile_id());
        for p in &plugins {
            let origin = if bootstrap.default_plugins.contains(p) {
                "shared"
            } else if bootstrap
                .features
                .values()
                .any(|f| f.plugin_for(agent) == Some(p.as_str()))
            {
                "feature"
            } else {
                agent.profile_id()
            };
            println!("  plugin    {p}  [{origin}]");
        }
        for s in &skills {
            let origin = if bootstrap.default_skills.contains(s) {
                "shared"
            } else {
                agent.profile_id()
            };
            println!("  skill     {}  [{origin}]", s.display());
        }
        for s in &subagents {
            println!("  subagent  {}", subagent_label(s));
        }
        for p in &prompts {
            let origin = if bootstrap.default_prompt_components.contains(p) {
                "shared"
            } else if bootstrap
                .features
                .values()
                .any(|f| f.prompt_component_for(agent) == Some(p.as_str()))
            {
                "feature"
            } else {
                agent.profile_id()
            };
            println!("  prompt    {p}  [{origin}]");
        }
        if let Some(patch) = settings {
            // Keys only: values can be whole nested objects, and this is a
            // declaration listing, not a settings dump.
            let keys = match patch.as_object() {
                Some(o) => o.keys().cloned().collect::<Vec<_>>().join(", "),
                None => "<document>".to_string(),
            };
            println!("  settings  {keys}  [{}]", agent.profile_id());
        }
    }
    Ok(())
}

fn subagent_label(def: &crate::bootstrap::SubagentDef) -> String {
    match def {
        crate::bootstrap::SubagentDef::File(path) => path.clone(),
        crate::bootstrap::SubagentDef::Inline { name, .. } => format!("{name} (inline)"),
    }
}

pub fn run(args: BootstrapArgs) -> Result<()> {
    match args.command {
        BootstrapCommand::Plugin { action } => run_action(Kind::Plugin, action),
        BootstrapCommand::Skill { action } => run_action(Kind::Skill, action),
        BootstrapCommand::Subagent { action } => run_action(Kind::Subagent, action),
        BootstrapCommand::Prompt { action } => run_action(Kind::Prompt, action),
        BootstrapCommand::List { agent } => run_list(agent),
        BootstrapCommand::Sync => sync(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_paths_match_the_config_schema() {
        assert_eq!(key_path(Kind::Plugin, None), ["bootstrap", "default_plugins"]);
        assert_eq!(
            key_path(Kind::Skill, Some(Agent::Pi)),
            ["bootstrap", "agents", "pi", "additional_skills"]
        );
        assert_eq!(
            key_path(Kind::Prompt, Some(Agent::Claude)),
            ["bootstrap", "agents", "claude", "additional_prompt_components"]
        );
    }

    #[test]
    fn unknown_agents_are_rejected_with_the_known_list() {
        let err = parse_agent("nope").unwrap_err().to_string();
        assert!(err.contains("nope"), "{err}");
        assert!(err.contains("pi"), "names the known agents: {err}");
    }

    #[test]
    fn entries_match_in_string_and_map_form() {
        let string: serde_yaml::Value = serde_yaml::from_str("./skills/x").unwrap();
        assert!(entry_matches(&string, "./skills/x"));
        assert!(!entry_matches(&string, "./skills/y"));

        let mapped: serde_yaml::Value =
            serde_yaml::from_str("path: ./skills/x\nhooks:\n  turn-done:\n    - command: ls")
                .unwrap();
        assert!(entry_matches(&mapped, "./skills/x"), "map form matches on path");

        let remote: serde_yaml::Value = serde_yaml::from_str("url: https://x/y\nref: main").unwrap();
        assert!(entry_matches(&remote, "https://x/y"));
    }

    #[test]
    fn an_emptied_list_removes_the_key() {
        assert!(render("default_plugins", &[]).unwrap().is_none());
        let one = vec![serde_yaml::Value::String("a".into())];
        assert_eq!(
            render("default_plugins", &one).unwrap().as_deref(),
            Some("default_plugins:\n- a")
        );
    }
}
