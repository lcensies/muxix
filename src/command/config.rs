//! Global configuration management commands.

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Args)]
pub struct ConfigArgs {
    #[command(subcommand)]
    pub command: ConfigCommand,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Open the global configuration file in your editor ($VISUAL, $EDITOR, or vi)
    Edit,
    /// Print the path to the global configuration file
    Path,
    /// Print the default configuration reference with all options documented
    Reference,
    /// Print the effective config after includes, layer merging, and profiles
    Resolve {
        /// Resolve this file on its own instead of the ambient global/project
        /// pair. Useful for checking a generated config on any machine.
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        /// Output format
        #[arg(long, value_enum, default_value_t = ResolveFormat::Yaml)]
        format: ResolveFormat,
        /// Annotate each key with the layer that set its final value
        #[arg(long)]
        explain: bool,
    },
    /// Inspect and edit which coding agent CLI each project uses
    Agent {
        #[command(subcommand)]
        command: AgentCommand,
    },
    /// Check that the config resolves and every key is recognized
    Validate {
        /// Validate this file instead of the ambient global/project pair
        #[arg(long, value_name = "PATH")]
        file: Option<PathBuf>,
        /// Treat unrecognized keys as errors rather than warnings
        #[arg(long)]
        strict: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// List the default agent and every path rule, in evaluation order
    List,
    /// Show which agent resolves for a directory, and what decided it
    Which {
        /// Directory to resolve for (default: current directory)
        dir: Option<PathBuf>,
    },
    /// Set the agent for a path rule, or the global default
    Set {
        /// Agent name (a key of `agents`) or a bare command
        agent: String,
        /// Match project roots with this regex
        #[arg(long, value_name = "REGEX", group = "target")]
        r#match: Option<String>,
        /// Match the root of this tracked project
        #[arg(long, value_name = "NAME", group = "target")]
        project: Option<String>,
        /// Match this directory and everything under it
        #[arg(long, value_name = "DIR", group = "target")]
        path: Option<PathBuf>,
        /// Set the global `agent:` default instead of a rule
        #[arg(long, group = "target")]
        default: bool,
    },
    /// Remove a path rule
    Unset {
        /// Remove the rule with this exact pattern
        #[arg(long, value_name = "REGEX", group = "target")]
        r#match: Option<String>,
        /// Remove the rule generated for this tracked project
        #[arg(long, value_name = "NAME", group = "target")]
        project: Option<String>,
        /// Remove the rule at this index (see `config agent list`)
        #[arg(long, value_name = "N", group = "target")]
        index: Option<usize>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ResolveFormat {
    Yaml,
    Json,
}

pub fn run(args: ConfigArgs) -> Result<()> {
    match args.command {
        ConfigCommand::Edit => run_edit(),
        ConfigCommand::Path => run_path(),
        ConfigCommand::Reference => run_reference(),
        ConfigCommand::Resolve {
            file,
            format,
            explain,
        } => run_resolve(file.as_deref(), format, explain),
        ConfigCommand::Validate { file, strict } => run_validate(file.as_deref(), strict),
        ConfigCommand::Agent { command } => run_agent(command),
    }
}

/// Read the global config file on its own (no includes, no project layer):
/// this is the file `set`/`unset` write back to.
fn read_global_value() -> Result<(PathBuf, serde_yaml::Value)> {
    let path = crate::config::global_config_path()
        .or_else(|| crate::xdg::config_dir().ok().map(|d| d.join("config.yaml")))
        .context("Could not determine the global config path")?;
    let value = match fs::read_to_string(&path) {
        Ok(s) => serde_yaml::from_str(&s)
            .with_context(|| format!("Failed to parse {}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => serde_yaml::Value::Null,
        Err(e) => return Err(e).with_context(|| format!("Failed to read {}", path.display())),
    };
    Ok((path, value))
}

fn read_rules(value: &serde_yaml::Value) -> Result<Vec<crate::config::AgentRule>> {
    match value.get("agent_rules") {
        Some(v) => serde_yaml::from_value(v.clone()).context("Failed to parse agent_rules"),
        None => Ok(Vec::new()),
    }
}

fn write_rules(path: &std::path::Path, rules: &[crate::config::AgentRule]) -> Result<()> {
    let block = if rules.is_empty() {
        None
    } else {
        let body = serde_yaml::to_string(rules)?;
        Some(format!("agent_rules:\n{}", body.trim_end()))
    };
    crate::config::edit::edit_file(path, "agent_rules", block.as_deref())
}

/// A pattern matching `root` and everything beneath it.
fn anchored_pattern(root: &std::path::Path) -> String {
    format!("^{}(/|$)", regex::escape(&root.to_string_lossy()))
}

fn project_root(name: &str) -> Result<PathBuf> {
    let registry = crate::projects::registry::Registry::load()?;
    registry
        .projects
        .iter()
        .find(|p| p.name == name)
        .map(|p| p.root.clone())
        .with_context(|| format!("No tracked project named '{name}' (see `muxix project list`)"))
}

fn run_agent(command: AgentCommand) -> Result<()> {
    match command {
        AgentCommand::List => {
            let (path, value) = read_global_value()?;
            let rules = read_rules(&value)?;
            let default = value
                .get("agent")
                .and_then(|v| v.as_str())
                .unwrap_or("claude (built-in default)");
            println!("{}", path.display());
            println!("default agent: {default}");
            if rules.is_empty() {
                println!("no agent_rules");
            } else {
                println!("rules (first match wins):");
                for (i, rule) in rules.iter().enumerate() {
                    let invalid = rule.compile().err().map(|e| format!("  [invalid regex: {e}]"));
                    println!(
                        "  {i}  {:<40} -> {}{}",
                        rule.pattern,
                        rule.agent,
                        invalid.unwrap_or_default()
                    );
                }
            }
            Ok(())
        }
        AgentCommand::Which { dir } => {
            let dir = match dir {
                Some(d) => d,
                None => std::env::current_dir()?,
            };
            let config = crate::config::Config::load_from(&dir, None)?;
            let agent = config.agent.as_deref().unwrap_or("claude");
            let source = config
                .agent_source
                .map(|s| s.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            println!("{}: {agent}  ({source})", dir.display());
            Ok(())
        }
        AgentCommand::Set {
            agent,
            r#match,
            project,
            path: dir,
            default,
        } => {
            let (config_path, value) = read_global_value()?;
            if default {
                crate::config::edit::edit_file(
                    &config_path,
                    "agent",
                    Some(&format!("agent: {agent}")),
                )?;
                println!("default agent -> {agent} ({})", config_path.display());
                return Ok(());
            }

            let pattern = match (r#match, project, dir) {
                (Some(p), _, _) => p,
                (_, Some(name), _) => anchored_pattern(&project_root(&name)?),
                (_, _, Some(d)) => anchored_pattern(&d.canonicalize().unwrap_or(d)),
                _ => bail!("Pass one of --match, --project, --path, or --default"),
            };
            regex::Regex::new(&pattern)
                .with_context(|| format!("Invalid regex: {pattern}"))?;

            let mut rules = read_rules(&value)?;
            match rules.iter_mut().find(|r| r.pattern == pattern) {
                Some(existing) => existing.agent = agent.clone(),
                None => rules.push(crate::config::AgentRule {
                    pattern: pattern.clone(),
                    agent: agent.clone(),
                }),
            }
            write_rules(&config_path, &rules)?;
            println!("{pattern} -> {agent} ({})", config_path.display());
            Ok(())
        }
        AgentCommand::Unset {
            r#match,
            project,
            index,
        } => {
            let (config_path, value) = read_global_value()?;
            let mut rules = read_rules(&value)?;
            let pattern = match (r#match, project) {
                (Some(p), _) => Some(p),
                (_, Some(name)) => Some(anchored_pattern(&project_root(&name)?)),
                _ => None,
            };
            let removed = match (pattern, index) {
                (Some(p), _) => {
                    let before = rules.len();
                    rules.retain(|r| r.pattern != p);
                    if rules.len() == before {
                        bail!("No rule with pattern {p}");
                    }
                    p
                }
                (None, Some(i)) => {
                    if i >= rules.len() {
                        bail!("No rule at index {i} ({} rule(s))", rules.len());
                    }
                    rules.remove(i).pattern
                }
                _ => bail!("Pass one of --match, --project, or --index"),
            };
            write_rules(&config_path, &rules)?;
            println!("removed {removed} ({})", config_path.display());
            Ok(())
        }
    }
}

/// Print the effective config.
///
/// Nothing is written to stdout unless resolution fully succeeds, so a caller
/// piping this into a file never captures a half-resolved config.
fn run_resolve(
    file: Option<&std::path::Path>,
    format: ResolveFormat,
    explain: bool,
) -> Result<()> {
    let profile = crate::config::profiles::cli_profile();
    let (resolved, layers) = match file {
        // A named file is judged on its own, so a generated config resolves the
        // same way on any machine regardless of what is installed there.
        Some(path) => (
            crate::config::Config::resolve_file(path, profile, explain)?,
            Vec::new(),
        ),
        None => {
            let start_dir = std::env::current_dir().unwrap_or_default();
            crate::config::Config::resolve_value(&start_dir, None, profile, explain)?
        }
    };

    for warning in &resolved.warnings {
        eprintln!("warning: {} ({})", warning.message, warning.source);
    }

    let rendered = if explain {
        let provenance = resolved
            .provenance
            .as_ref()
            .expect("provenance is tracked when --explain is set");
        render_explained(&resolved.value, provenance, &layers, format)?
    } else {
        match format {
            ResolveFormat::Yaml => serde_yaml::to_string(&resolved.value)?,
            ResolveFormat::Json => serde_json::to_string_pretty(&resolved.value)?,
        }
    };

    print!("{rendered}");
    Ok(())
}

/// Render the resolved config with per-key attribution.
///
/// YAML gets a trailing `# from <source>` comment per line, which keeps the
/// output readable and still valid YAML. JSON gets a separate `provenance`
/// object, since JSON has no comments.
fn render_explained(
    value: &serde_yaml::Value,
    provenance: &crate::config::resolve::Provenance,
    layers: &[crate::config::LayerInfo],
    format: ResolveFormat,
) -> Result<String> {
    let source_of = |layer_id: &str| -> String {
        layers
            .iter()
            .find(|l| l.id == layer_id)
            .map(|l| l.source.clone())
            .unwrap_or_else(|| layer_id.to_string())
    };

    match format {
        ResolveFormat::Json => {
            let annotated: serde_json::Map<String, serde_json::Value> = provenance
                .iter()
                .map(|(key, layer)| {
                    (
                        key.clone(),
                        serde_json::json!({ "layer": layer, "source": source_of(layer) }),
                    )
                })
                .collect();
            let doc = serde_json::json!({
                "config": value,
                "provenance": annotated,
            });
            Ok(format!("{}\n", serde_json::to_string_pretty(&doc)?))
        }
        ResolveFormat::Yaml => {
            let plain = serde_yaml::to_string(value)?;
            let mut out = String::with_capacity(plain.len() * 2);
            // Track the dotted path by indentation so each line can be matched
            // against the provenance map.
            let mut stack: Vec<(usize, String)> = Vec::new();
            for line in plain.lines() {
                let trimmed = line.trim_start();
                let indent = line.len() - trimmed.len();
                let key = trimmed
                    .split_once(':')
                    .map(|(k, _)| k.trim())
                    .filter(|k| !k.is_empty() && !k.starts_with('-') && !k.starts_with('#'));

                if let Some(key) = key {
                    while stack.last().is_some_and(|(i, _)| *i >= indent) {
                        stack.pop();
                    }
                    let path = match stack.last() {
                        Some((_, parent)) => format!("{parent}.{key}"),
                        None => key.to_string(),
                    };
                    stack.push((indent, path.clone()));

                    if let Some(layer) = provenance.get(&path) {
                        out.push_str(line);
                        out.push_str(&format!("  # from {}\n", source_of(layer)));
                        continue;
                    }
                }
                out.push_str(line);
                out.push('\n');
            }
            Ok(out)
        }
    }
}

/// Validate a config without applying it.
///
/// With `--file`, the named file is judged on its own — the ambient global and
/// project configs are ignored, so a rendered file validates the same way on
/// any machine. Without it, the effective config is validated as it would
/// actually load.
fn run_validate(file: Option<&std::path::Path>, strict: bool) -> Result<()> {
    let profile = crate::config::profiles::cli_profile();

    let (value, warnings) = match file {
        Some(path) => {
            let resolved = crate::config::Config::resolve_file(path, profile, false)?;
            (resolved.value, resolved.warnings)
        }
        None => {
            let start_dir = std::env::current_dir().unwrap_or_default();
            let (resolved, _) =
                crate::config::Config::resolve_value(&start_dir, None, profile, false)?;
            (resolved.value, resolved.warnings)
        }
    };

    for warning in &warnings {
        eprintln!("warning: {} ({})", warning.message, warning.source);
    }

    check_value(&value, strict, "config")?;

    // Every declared profile is checked in turn: a profile that only breaks
    // when selected is a trap that surfaces on someone else's machine.
    if let Some(path) = file {
        let mut failures = Vec::new();
        for name in crate::config::Config::declared_profiles(path)? {
            let resolved = match crate::config::Config::resolve_file(path, Some(&name), false) {
                Ok(r) => r,
                Err(e) => {
                    failures.push(format!("profile `{name}`: {e}"));
                    continue;
                }
            };
            if let Err(e) = check_value(&resolved.value, strict, &format!("profile `{name}`")) {
                failures.push(e.to_string());
            }
        }
        if !failures.is_empty() {
            bail!("{}", failures.join("\n"));
        }
    }

    println!("config is valid");
    Ok(())
}

/// Deserialize `value` as a `Config` and report unrecognized keys.
fn check_value(value: &serde_yaml::Value, strict: bool, what: &str) -> Result<()> {
    // Deserializing is the schema check: it catches wrong types, bad enum
    // variants, and missing required fields.
    let config: crate::config::Config = serde_yaml::from_value(value.clone())
        .map_err(|e| anyhow::anyhow!("{what} is not valid: {e}"))?;

    // A rule whose regex does not compile is skipped at load time with a
    // stderr note; validate is where it should be an outright failure.
    for (i, rule) in config.agent_rules.iter().enumerate() {
        if let Err(e) = rule.compile() {
            bail!("{what}: agent_rules[{i}] pattern {:?} is not a valid regex: {e}", rule.pattern);
        }
    }

    // Unknown keys are reported separately: serde ignores them by default, and
    // silently dropping a misspelled key is the failure mode this catches.
    let unknown = crate::config::unknown_top_level_keys(value);
    if !unknown.is_empty() {
        let list = unknown.join(", ");
        if strict {
            bail!("{what} has unrecognized key(s): {list}");
        }
        eprintln!("warning: {what} has unrecognized key(s): {list}");
    }
    Ok(())
}

fn run_edit() -> Result<()> {
    let config_path =
        crate::config::global_config_path().context("Could not determine home directory")?;

    // Ensure directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }

    // Create default config if it doesn't exist
    if !config_path.exists() {
        fs::write(&config_path, DEFAULT_GLOBAL_CONFIG)
            .with_context(|| format!("Failed to create {}", config_path.display()))?;
        println!("Created {}", config_path.display());
    }

    // Determine editor: $VISUAL -> $EDITOR -> vi
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    // Split editor string to handle values like "code --wait"
    let parts: Vec<&str> = editor.split_whitespace().collect();
    let (cmd, args) = parts.split_first().context("Editor variable is empty")?;

    let status = Command::new(cmd)
        .args(args)
        .arg(&config_path)
        .status()
        .with_context(|| format!("Failed to open editor '{}'", editor))?;

    if !status.success() {
        bail!("Editor '{}' exited with non-zero status", editor);
    }

    Ok(())
}

fn run_path() -> Result<()> {
    let config_path =
        crate::config::global_config_path().context("Could not determine home directory")?;
    println!("{}", config_path.display());
    Ok(())
}

fn run_reference() -> Result<()> {
    print!("{}", crate::config::EXAMPLE_PROJECT_CONFIG);
    Ok(())
}

const DEFAULT_GLOBAL_CONFIG: &str = r#"# muxix global configuration
# Settings here apply to all projects. Project-specific .muxix.yaml overrides these.
# See: https://muxix.dev/guide/configuration

# nerdfont: true
# agent: claude
#
# Per-project agent, first match wins (see `muxix config agent --help`):
# agent_rules:
#   - match: "^~/repos/work/"
#     agent: opencode
# merge_strategy: rebase
# merge_keep: true
#
# panes:
#   - command: <agent>
#     focus: true
#   - split: horizontal
#
# sandbox:
#   host_commands: ["just", "cargo"]
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_global_config_is_valid_yaml() {
        let result: Result<crate::config::Config, _> = serde_yaml::from_str(DEFAULT_GLOBAL_CONFIG);
        assert!(
            result.is_ok(),
            "Default global config is not valid YAML: {:?}",
            result.err()
        );
    }
}
