//! Agent bootstrap configuration and installation.
//!
//! Configures plugins, skills, and prompts uniformly across agents
//! through a single manifest in `.workmux.yaml`.

#![allow(dead_code)]

use anyhow::{Context, Result};
use console::style;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::agent::setup::Agent;

/// A source for skills or plugins: either a local path or a remote URL.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Source {
    /// Local file path (relative to project root or absolute)
    LocalPath(String),
    /// Remote source: URL with optional ref/tag/branch for git repos
    Remote {
        url: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        r#ref: Option<String>,
    },
}

impl Source {
    /// Check if this is a git repository URL.
    pub fn is_git_url(&self) -> bool {
        match self {
            Source::LocalPath(_) => false,
            Source::Remote { url, .. } => {
                url.starts_with("https://github.com/")
                    || url.starts_with("git@github.com:")
                    || url.contains(".git")
            }
        }
    }

    /// Get the display name for this source.
    pub fn display(&self) -> String {
        match self {
            Source::LocalPath(p) => p.clone(),
            Source::Remote { url, r#ref } => {
                if let Some(r) = r#ref {
                    format!("{} ({})", url, r)
                } else {
                    url.clone()
                }
            }
        }
    }
}

/// Agent-agnostic hook events a skill may bind to.
///
/// Deliberately the same two moments the pipeline already models
/// (`signal_support`): translating them per agent is a lookup, not a design
/// question, and adding an event here forces the per-agent decision there.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HookEvent {
    SessionReady,
    TurnDone,
}

impl HookEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            HookEvent::SessionReady => "session-ready",
            HookEvent::TurnDone => "turn-done",
        }
    }

    pub fn parse(name: &str) -> Option<HookEvent> {
        match name {
            "session-ready" => Some(HookEvent::SessionReady),
            "turn-done" => Some(HookEvent::TurnDone),
            _ => None,
        }
    }
}

impl serde::Serialize for HookEvent {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for HookEvent {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        HookEvent::parse(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown hook event `{raw}`; valid events: session-ready, turn-done"
            ))
        })
    }
}

/// One command a skill binds to a hook event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookSpec {
    /// The command line to run. May reference `{{ skill_install_dir }}`,
    /// rendered per agent to the skill's installed directory.
    pub command: String,
    /// Optional sha256 of the script the command runs, verified against the
    /// *installed* copy before the hook is written. Mismatch fails closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// A skill to install, optionally with hooks bound to agent lifecycle events.
///
/// Hooks live on the skill entry rather than in a separate block so a skill is
/// self-contained -- its code, its instructions, and when it fires travel
/// together -- and so `{{ skill_install_dir }}` unambiguously means *this*
/// skill's directory.
#[derive(Debug, Clone, PartialEq)]
pub struct SkillEntry {
    pub source: Source,
    /// Global-only: stripped from project-declared entries before merging. A
    /// shipped SKILL.md is text an agent may act on; a hook is a command the
    /// harness will run unattended, and cloning a repo must not grant that.
    pub hooks: std::collections::BTreeMap<HookEvent, Vec<HookSpec>>,
    /// What the skill needs to run (npm packages, executables). Not
    /// global-only: a project may declare plugins and MCP commands already,
    /// which carry the same trust.
    pub requires: crate::deps::Requires,
}

impl SkillEntry {
    pub fn display(&self) -> String {
        self.source.display()
    }
}

impl From<Source> for SkillEntry {
    fn from(source: Source) -> Self {
        SkillEntry {
            source,
            hooks: Default::default(),
            requires: Default::default(),
        }
    }
}

impl serde::Serialize for SkillEntry {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        if self.hooks.is_empty() && self.requires.is_empty() {
            return self.source.serialize(s);
        }
        let mut map = s.serialize_map(None)?;
        match &self.source {
            Source::LocalPath(p) => map.serialize_entry("path", p)?,
            Source::Remote { url, r#ref } => {
                map.serialize_entry("url", url)?;
                if let Some(r) = r#ref {
                    map.serialize_entry("ref", r)?;
                }
            }
        }
        if !self.hooks.is_empty() {
            map.serialize_entry("hooks", &self.hooks)?;
        }
        if !self.requires.is_empty() {
            map.serialize_entry("requires", &self.requires)?;
        }
        map.end()
    }
}

impl<'de> serde::Deserialize<'de> for SkillEntry {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;

        // Three accepted shapes: a bare string, the existing {url, ref} table,
        // and {path|url, hooks}. Deserialized via Value so the string form
        // stays exactly as permissive as it was when the field was Vec<Source>.
        let value = serde_yaml::Value::deserialize(d)?;
        match value {
            serde_yaml::Value::String(path) => Ok(Source::LocalPath(path).into()),
            serde_yaml::Value::Mapping(map) => {
                let get = |k: &str| map.get(serde_yaml::Value::String(k.to_string()));
                let as_str = |v: Option<&serde_yaml::Value>| {
                    v.and_then(|v| v.as_str()).map(str::to_owned)
                };

                let source = match (as_str(get("path")), as_str(get("url"))) {
                    (Some(_), Some(_)) => {
                        return Err(D::Error::custom(
                            "skill entry sets both `path` and `url`; use one",
                        ));
                    }
                    (Some(path), None) => Source::LocalPath(path),
                    (None, Some(url)) => Source::Remote {
                        url,
                        r#ref: as_str(get("ref")),
                    },
                    (None, None) => {
                        return Err(D::Error::custom(
                            "skill entry needs `path` or `url`",
                        ));
                    }
                };

                let hooks = match get("hooks") {
                    None => Default::default(),
                    Some(raw) => serde_yaml::from_value(raw.clone())
                        .map_err(|e| D::Error::custom(format!("skill hooks: {e}")))?,
                };

                let requires = match get("requires") {
                    None => Default::default(),
                    Some(raw) => serde_yaml::from_value(raw.clone())
                        .map_err(|e| D::Error::custom(format!("skill requires: {e}")))?,
                };

                Ok(SkillEntry { source, hooks, requires })
            }
            other => Err(D::Error::custom(format!(
                "skill entry must be a string or a table, got {other:?}"
            ))),
        }
    }
}

/// A subagent definition: either a path to a markdown file (frontmatter +
/// prompt body — the default form) or an inline definition rendered to that
/// same format at install time.
///
/// ```yaml
/// default_subagents:
///   - ./agents/reviewer.md          # file (default)
///   - name: planner                 # inline
///     description: Plans work before execution
///     prompt: |
///       You are a planning specialist...
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SubagentDef {
    /// Path to a `.md` file, relative to project root or absolute.
    /// The subagent name is the file stem.
    File(String),
    /// Inline definition; serialized to markdown frontmatter on install.
    Inline {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        prompt: String,
        /// Optional tool allowlist, rendered as a comma-separated list.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tools: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
}

impl SubagentDef {
    /// Display name for status output.
    pub fn display(&self) -> String {
        match self {
            SubagentDef::File(p) => p.clone(),
            SubagentDef::Inline { name, .. } => format!("{name} (inline)"),
        }
    }

    /// Resolve to `(name, markdown content)`.
    pub fn resolve(&self, project_root: &Path) -> Result<(String, String)> {
        match self {
            SubagentDef::File(path) => {
                let resolved = if Path::new(path).is_absolute() {
                    PathBuf::from(path)
                } else {
                    project_root.join(path)
                };
                let content = fs::read_to_string(&resolved).with_context(|| {
                    format!("Failed to read subagent file {}", resolved.display())
                })?;
                let name = resolved
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .filter(|n| !n.is_empty())
                    .ok_or_else(|| {
                        anyhow::anyhow!("Cannot derive a subagent name from path: {path}")
                    })?
                    .to_string();
                Ok((name, content))
            }
            SubagentDef::Inline {
                name,
                description,
                prompt,
                tools,
                model,
            } => {
                if name.trim().is_empty() {
                    anyhow::bail!("Inline subagent has an empty name");
                }
                // Build frontmatter via serde_yaml so descriptions with
                // colons/quotes stay valid YAML.
                let mut fm = serde_yaml::Mapping::new();
                fm.insert("name".into(), name.clone().into());
                if let Some(d) = description {
                    fm.insert("description".into(), d.clone().into());
                }
                if !tools.is_empty() {
                    fm.insert("tools".into(), tools.join(", ").into());
                }
                if let Some(m) = model {
                    fm.insert("model".into(), m.clone().into());
                }
                let yaml = serde_yaml::to_string(&fm)
                    .context("Failed to serialize subagent frontmatter")?;
                Ok((name.clone(), format!("---\n{yaml}---\n\n{prompt}")))
            }
        }
    }
}

/// Per-agent bootstrap overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentBootstrapOverrides {
    /// Additional plugins for this agent only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_plugins: Vec<String>,

    /// Additional skills for this agent only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_skills: Vec<SkillEntry>,

    /// Additional subagents for this agent only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_subagents: Vec<SubagentDef>,

    /// Provider this agent resolves unqualified model names against.
    ///
    /// Lets one config drive a personal agent and a work agent side by side:
    /// point each at its own provider and a shared `model: haiku` resolves to
    /// that provider's id for each. A `<provider>/<name>` spec still wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_provider: Option<String>,

    /// Per-subagent `model:` overrides for this agent, keyed by subagent name.
    ///
    /// The spec replaces the subagent's frontmatter model (inserting it when
    /// absent) and then resolves like any other spec: against the `providers:`
    /// registry when it matches, verbatim otherwise — so a host-native spec
    /// like `corp/claude-haiku-4-5` passes straight through.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub subagent_models: BTreeMap<String, String>,

    /// Additional prompt components for this agent only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_prompt_components: Vec<String>,

    /// Prompt components to exclude for this agent (overrides defaults and additionals).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_prompt_components: Vec<String>,

    /// RFC 7386 merge patch applied to the agent's own settings file, so
    /// preferences that are not harness items (pi's `defaultTools`,
    /// `modelRoles`, …) are declared in config instead of hand-edited per
    /// machine. `null` deletes a key; unmentioned keys — including ones the
    /// agent writes itself — survive.
    ///
    /// Removing a declaration does not restore the previous value: workmux
    /// cannot know what it should be. Use an explicit `null` to remove a key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
}

/// A named capability that can be implemented differently per agent.
///
/// A feature decouples *what* you want (e.g. "ponytail") from *how* each agent
/// provides it. For an agent that has a dedicated plugin, the feature resolves
/// to that plugin spec; for every other agent it falls back to `default`, a
/// prompt-component name merged into the agent's system prompt.
///
/// ```yaml
/// features:
///   ponytail:
///     pi: git:github.com/DietrichGebert/ponytail   # pi installs a plugin
///     omp: pi-ponytail@marketplace                 # omp installs its own spec
///     default: ponytail                            # everyone else gets the prompt
/// ```
///
/// Agent keys accept either the short id (`pi`, `claude`) or the display name
/// lowercased (`claude code`) — see [`Agent::matches_config_key`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeatureConfig {
    /// Prompt-component name used for any agent without a specific plugin impl.
    /// `None` means the feature is plugin-only (agents without an impl skip it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,

    /// Agent-specific plugin specs, keyed by agent id. Captured via `flatten`
    /// so the feature reads as a flat `<agent>: <plugin-spec>` map alongside the
    /// reserved `default` key.
    #[serde(flatten, default)]
    pub agent_plugins: BTreeMap<String, String>,
}

impl FeatureConfig {
    /// The plugin spec this feature provides for `agent`, if any.
    pub fn plugin_for(&self, agent: Agent) -> Option<&str> {
        self.agent_plugins
            .iter()
            .find(|(k, _)| agent.matches_config_key(k))
            .map(|(_, v)| v.as_str())
    }

    /// The prompt-component fallback for `agent`: the configured `default`, but
    /// only when this agent has no dedicated plugin impl.
    pub fn prompt_component_for(&self, agent: Agent) -> Option<&str> {
        if self.plugin_for(agent).is_some() {
            return None;
        }
        self.default.as_deref()
    }
}

/// Bootstrap configuration for all agents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BootstrapConfig {
    /// Default plugins to install for all agents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_plugins: Vec<String>,

    /// Named features resolved per-agent to a plugin or a prompt component.
    /// Keeps agent-agnostic intent ("enable ponytail") separate from the
    /// agent-specific wiring.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub features: BTreeMap<String, FeatureConfig>,

    /// Hooks not tied to any skill, bound to agent lifecycle events.
    ///
    /// Global-only, like skill hooks: a hook is a command run unattended on
    /// every turn, so enabling one is the machine owner's decision.
    /// `{{ skill_install_dir }}` has no meaning here and is a render error.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hooks: BTreeMap<HookEvent, Vec<HookSpec>>,

    /// Default skills to install for all agents.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_skills: Vec<SkillEntry>,

    /// npm global prefix for `requires.npm` installs. Default `~/.local`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npm_prefix: Option<String>,

    /// Prune every package under the npm prefix that no entity declares, not
    /// only the ones workmux installed. Off: hand-installed packages survive.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub deps_strict: bool,

    /// Variables available to skill templates (minijinja), resolved per host
    /// agent at install time. A scalar applies to every agent; a map is keyed
    /// by agent id (same key forms as `agents:`), with optional `default`.
    /// Built-ins `agent` (canonical id) and `agent_name` (display name) are
    /// always present and cannot be overridden.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub template_vars: BTreeMap<String, TemplateVar>,

    /// Color theme applied to each agent's own config by `workmux setup`.
    ///
    /// A scalar sets the same theme everywhere; a map keys it per agent (same
    /// key forms as `agents:`) with an optional `default`, since theme names
    /// are not portable — OpenCode ships `catppuccin`, Claude Code only its own
    /// `dark`/`light` variants:
    ///
    /// ```yaml
    /// theme:
    ///   claude code: dark
    ///   default: catppuccin
    /// ```
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<TemplateVar>,

    /// Default subagents to install for all agents that support them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_subagents: Vec<SubagentDef>,

    /// Default prompt components to merge for all agents.
    /// Components are loaded from `.workmux/prompt-components/` directory.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_prompt_components: Vec<String>,

    /// Per-agent overrides.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, AgentBootstrapOverrides>,

    /// Pi-specific bootstrap configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pi: Option<crate::agent::setup::pi::PiBootstrapConfig>,
}

/// A skill-template variable: one value for all agents, or a per-agent map.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TemplateVar {
    Scalar(String),
    PerAgent(BTreeMap<String, String>),
}

impl BootstrapConfig {
    /// Build the minijinja context for rendering skills for `agent`:
    /// built-ins (`agent`, `agent_name`) plus `template_vars` resolved for
    /// this agent. Per-agent maps fall back to a `default` key; a var with
    /// no value for this agent and no default is an error, not a guess.
    pub fn skill_template_context(
        &self,
        agent: Agent,
        install_dir: Option<&Path>,
    ) -> Result<serde_json::Value> {
        let mut ctx = serde_json::Map::new();
        ctx.insert("agent".into(), agent.profile_id().into());
        ctx.insert("agent_name".into(), agent.name().into());
        // The *destination*, not the source: the rendered SKILL.md and any
        // scripts the skill ships end up side by side there, and the path
        // differs per agent -- which is exactly why it must be a variable.
        if let Some(dir) = install_dir {
            ctx.insert(
                "skill_install_dir".into(),
                dir.display().to_string().into(),
            );
        }
        for (key, var) in &self.template_vars {
            if key == "agent" || key == "agent_name" || key == "skill_install_dir" {
                anyhow::bail!("template_vars: `{key}` is a built-in and cannot be overridden");
            }
            let value = match var {
                TemplateVar::Scalar(s) => s.clone(),
                TemplateVar::PerAgent(map) => map
                    .iter()
                    .find(|(k, _)| agent.matches_config_key(k))
                    .map(|(_, v)| v)
                    .or_else(|| map.get("default"))
                    .with_context(|| {
                        format!(
                            "template_vars: `{key}` has no value for agent `{}` and no `default`",
                            agent.profile_id()
                        )
                    })?
                    .clone(),
            };
            ctx.insert(key.clone(), value.into());
        }
        Ok(serde_json::Value::Object(ctx))
    }

    /// Per-agent overrides for `agent`, keyed by either the short id (`claude`)
    /// or the lowercased display name (`claude code`) — the same two forms
    /// `features` accepts, via [`Agent::matches_config_key`].
    fn overrides_for(&self, agent: Agent) -> Option<&AgentBootstrapOverrides> {
        self.agents
            .iter()
            .find(|(k, _)| agent.matches_config_key(k))
            .map(|(_, v)| v)
    }

    /// The provider this agent resolves unqualified model names against.
    pub fn default_provider_for(&self, agent: Agent) -> Option<&str> {
        self.overrides_for(agent)?.default_provider.as_deref()
    }

    /// The merge patch declared for this agent's own settings file.
    pub fn settings_for(&self, agent: Agent) -> Option<&serde_json::Value> {
        self.overrides_for(agent)?.settings.as_ref()
    }

    /// The theme configured for `agent`: the per-agent entry, else `default`,
    /// else the scalar form. `None` leaves the agent's own theme untouched.
    pub fn theme_for(&self, agent: Agent) -> Option<&str> {
        match self.theme.as_ref()? {
            TemplateVar::Scalar(s) => Some(s.as_str()),
            TemplateVar::PerAgent(map) => map
                .iter()
                .find(|(k, _)| agent.matches_config_key(k))
                .map(|(_, v)| v.as_str())
                .or_else(|| map.get("default").map(String::as_str)),
        }
    }

    /// Get the list of plugins for a specific agent.
    ///
    /// Combines (1) `default_plugins`, (2) per-agent `additional_plugins`, and
    /// (3) any `features` that provide a plugin impl for this agent.
    pub fn plugins_for(&self, agent: Agent) -> Vec<String> {
        let mut plugins = self.default_plugins.clone();
        if let Some(overrides) = self.overrides_for(agent) {
            plugins.extend(overrides.additional_plugins.clone());
        }
        for feature in self.features.values() {
            if let Some(spec) = feature.plugin_for(agent) {
                plugins.push(spec.to_string());
            }
        }
        plugins.sort();
        plugins.dedup();
        plugins
    }

    /// Get the list of skills for a specific agent.
    pub fn skills_for(&self, agent: Agent) -> Vec<Source> {
        self.skill_entries_for(agent)
            .into_iter()
            .map(|e| e.source)
            .collect()
    }

    /// Skills for an agent with their hook declarations intact.
    pub fn skill_entries_for(&self, agent: Agent) -> Vec<SkillEntry> {
        let mut skills = self.default_skills.clone();
        if let Some(overrides) = self.overrides_for(agent) {
            skills.extend(overrides.additional_skills.clone());
        }
        skills
    }

    /// Get the list of subagents for a specific agent.
    pub fn subagents_for(&self, agent: Agent) -> Vec<SubagentDef> {
        let mut subagents = self.default_subagents.clone();
        if let Some(overrides) = self.overrides_for(agent) {
            subagents.extend(overrides.additional_subagents.clone());
        }
        subagents
    }

    /// Get the list of prompt components for a specific agent.
    ///
    /// Combines (1) `default_prompt_components`, (2) per-agent
    /// `additional_prompt_components`, and (3) `features` that fall back to a
    /// `default` prompt component for this agent (i.e. have no plugin impl for
    /// it). Per-agent `disabled_prompt_components` filters the final set, so a
    /// feature default can still be suppressed for a specific agent.
    pub fn prompt_components_for(&self, agent: Agent) -> Vec<String> {
        let mut components = self.default_prompt_components.clone();
        for feature in self.features.values() {
            if let Some(component) = feature.prompt_component_for(agent) {
                components.push(component.to_string());
            }
        }
        if let Some(overrides) = self.overrides_for(agent) {
            components.extend(overrides.additional_prompt_components.clone());
            components.retain(|c| !overrides.disabled_prompt_components.contains(c));
        }
        components.sort();
        components.dedup();
        components
    }
}

/// Write the configured theme into `agent`'s own config file.
///
/// Each agent stores the theme in its own place and under its own key, so this
/// dispatches to the agent module. Agents with no known theme setting (Codex,
/// Copilot, pi, omp) are a no-op. Returns the status line to print, or `None`
/// when nothing changed.
pub fn apply_theme_for_agent(agent: Agent, config: &BootstrapConfig) -> Result<Option<String>> {
    use crate::agent::setup::{claude, gemini, opencode};

    let Some(theme) = config.theme_for(agent) else {
        return Ok(None);
    };
    let changed = match agent {
        Agent::Claude => claude::set_theme(theme)?,
        Agent::Gemini => gemini::set_theme(theme)?,
        Agent::OpenCode => opencode::set_theme(theme)?,
        Agent::Codex | Agent::Copilot | Agent::Pi | Agent::Omp => return Ok(None),
    };
    Ok(changed.then(|| format!("Set {} theme to {theme}", agent.name())))
}

/// Apply merged prompt components to an agent's configuration.
pub fn apply_prompt_to_agent(
    agent: Agent,
    config: &BootstrapConfig,
    project_root: &Path,
) -> Result<Option<String>> {
    let components = config.prompt_components_for(agent);

    if components.is_empty() {
        return Ok(None);
    }

    let (merged_prompt, _) = merge_prompt_components("", &components, project_root)?;

    if merged_prompt.trim().is_empty() {
        Ok(None)
    } else {
        Ok(Some(merged_prompt))
    }
}

/// Merge prompt components with an existing prompt, avoiding duplication.
/// Returns true if any new components were added.
pub fn merge_prompt_components(
    existing_prompt: &str,
    components: &[String],
    project_root: &Path,
) -> Result<(String, bool)> {
    let mut merged = existing_prompt.to_string();
    let mut added_any = false;

    for component_name in components {
        // A name resolves under the project's component dir; anything with a
        // separator is a path (absolute, `~`, or project-relative), so a
        // machine-global config can point at components outside the project.
        let component_path = if component_name.contains('/') {
            let p = crate::util::expand_tilde(component_name);
            if p.is_absolute() { p } else { project_root.join(p) }
        } else {
            project_root
                .join(".workmux/prompt-components")
                .join(format!("{}.md", component_name))
        };

        let Ok(content) = fs::read_to_string(&component_path) else {
            // Component file not found, skip silently
            continue;
        };

        // Check if component is already present (idempotent)
        if !merged.contains(&content) {
            if !merged.ends_with('\n') {
                merged.push('\n');
            }
            merged.push('\n');
            merged.push_str(&content);
            added_any = true;
        }
    }

    Ok((merged, added_any))
}

/// Resolve and apply bootstrap prompt components for an agent.
pub fn resolve_prompt_for_agent(
    agent: Agent,
    config: &BootstrapConfig,
    project_root: &Path,
) -> Result<Option<String>> {
    let components = config.prompt_components_for(agent);

    if components.is_empty() {
        return Ok(None);
    }

    // Start with an empty prompt and merge components
    let (merged_prompt, _) = merge_prompt_components("", &components, project_root)?;
    Ok(if merged_prompt.trim().is_empty() {
        None
    } else {
        Some(merged_prompt)
    })
}

/// Resolve a skill source to its local path.
pub fn resolve_skill_source(
    source: &Source,
    project_root: &Path,
    cache_dir: &Path,
    skill_name: &str,
) -> Result<PathBuf> {
    match source {
        Source::LocalPath(path) => {
            let resolved = if Path::new(path).is_absolute() {
                PathBuf::from(path)
            } else {
                project_root.join(path)
            };
            if !resolved.exists() {
                anyhow::bail!("Skill path does not exist: {}", resolved.display());
            }
            Ok(resolved)
        }
        Source::Remote {
            url: _url,
            r#ref: _ref,
        } => {
            // For now, just return a cache path for the skill.
            // Actual git clone/fetch logic would be implemented here.
            fs::create_dir_all(cache_dir).context("Failed to create skills cache directory")?;
            Ok(cache_dir.join(skill_name))
        }
    }
}

/// Outcome of installing one bootstrap skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillInstall {
    Installed(String),
    Updated(String),
    UpToDate(String),
    /// Not installed, with the reason why (e.g. remote sources are not
    /// fetched yet). Reported rather than raised so one bad entry does not
    /// abort the rest of `workmux setup`.
    Skipped(String, String),
}

/// Install the bootstrap skills configured for `agent` into that agent's
/// skills directory.
///
/// Local paths may point at either a skill directory (containing `SKILL.md`)
/// or the `SKILL.md` file itself; the skill name is the directory name in
/// both cases. Agents without a skills directory (Codex, Copilot, Gemini)
/// are a no-op. Unlike the bundled-skill installer, this overwrites without
/// prompting: the project config is the source of truth.
pub fn install_skills_for_agent(
    agent: Agent,
    config: &BootstrapConfig,
    project_root: &Path,
    dry_run: bool,
) -> Result<Vec<SkillInstall>> {
    let sources = config.skills_for(agent);
    if sources.is_empty() {
        return Ok(Vec::new());
    }

    let Some(base_dir) = crate::skills::skills_dir(agent) else {
        return Ok(Vec::new());
    };

    let cache_dir = crate::xdg::cache_dir()?.join("skills");
    let mut results = Vec::new();

    for source in &sources {
        let name = skill_name_from_source(source)?;

        if matches!(source, Source::Remote { .. }) {
            results.push(SkillInstall::Skipped(
                name,
                "remote skill sources are not fetched yet; use a local path".to_string(),
            ));
            continue;
        }

        let resolved = resolve_skill_source(source, project_root, &cache_dir, &name)?;

        // A source may name the SKILL.md file directly; install its directory.
        let src_dir = if resolved.is_file() {
            resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or(resolved.clone())
        } else {
            resolved
        };

        let src_skill_md = src_dir.join("SKILL.md");
        if !src_skill_md.exists() {
            anyhow::bail!("Skill source {} has no SKILL.md", src_dir.display());
        }

        let raw_skill_md = fs::read_to_string(&src_skill_md)
            .with_context(|| format!("Failed to read {}", src_skill_md.display()))?;
        let dest_dir = base_dir.join(&name);
        let skill_md = render_skill_template(&raw_skill_md, agent, config, Some(&dest_dir))
            .with_context(|| format!("Failed to render skill template {name}"))?;

        let existed = dest_dir.join("SKILL.md").exists();
        let unchanged = existed && skill_install_up_to_date(&src_dir, &dest_dir, &skill_md)?;

        if unchanged {
            results.push(SkillInstall::UpToDate(name));
            continue;
        }

        // The outcome is decided above, so `--check` reports exactly what a
        // real run would do without the two writes below ever happening.
        if !dry_run {
            copy_dir_recursive(&src_dir, &dest_dir).with_context(|| {
                format!("Failed to install skill {name} to {}", dest_dir.display())
            })?;
            fs::write(dest_dir.join("SKILL.md"), &skill_md)
                .with_context(|| format!("Failed to write rendered SKILL.md for {name}"))?;
        }

        results.push(if existed {
            SkillInstall::Updated(name)
        } else {
            SkillInstall::Installed(name)
        });
    }

    Ok(results)
}

/// Render a subagent's `model:` spec for a specific host agent.
///
/// A spec is a logical name (`haiku`), a provider-qualified name
/// (`bedrock/haiku`), or a concrete provider id — resolved against the
/// project's `providers:` registry. The same logical model is served under
/// different ids per provider, and each host agent wants a different form:
/// Claude Code takes a bare alias or id, OpenCode takes `provider/id`.
///
/// Provider precedence: the spec's own `<provider>/` prefix, then the agent's
/// `default_provider`, then a unique match across the whole registry.
///
/// Never guesses: an unqualified name that matches zero providers — or more
/// than one, with no default to disambiguate — passes through verbatim so the
/// host resolves it (and fails loudly) rather than binding to the wrong one.
pub fn render_model(
    spec: &str,
    registry: Option<&crate::model::ProviderRegistry>,
    agent: Agent,
    default_provider: Option<&str>,
) -> String {
    // pi subagents run under taskflow, which resolves tier roles via
    // `settings.json.modelRoles` rather than the workmux provider registry.
    // Map bare tier aliases before (and independent of) registry lookup.
    if agent == Agent::Pi {
        match spec {
            "haiku" => return "\"{{scout}}\"".to_string(),
            "sonnet" => return "\"{{builder}}\"".to_string(),
            "opus" => return "\"{{expert}}\"".to_string(),
            _ => {}
        }
    }

    let Some(registry) = registry else {
        return spec.to_string();
    };

    // `bedrock/haiku` pins the provider; otherwise fall back to the agent's
    // default, then to a registry-wide unique match.
    let (provider, name) = match spec.split_once('/') {
        Some((p, n)) => (Some(p), n),
        None => (default_provider, spec),
    };

    let mut matches = crate::model::resolve_all(registry, name);
    if let Some(p) = provider {
        matches.retain(|m| m.provider.eq_ignore_ascii_case(p));
    }
    let [resolved] = matches.as_slice() else {
        return spec.to_string();
    };

    match agent {
        Agent::OpenCode => format!("{}/{}", resolved.provider, resolved.model.id),
        _ => resolved.model.id.clone(),
    }
}

/// Rewrite the `model:` value in a subagent's frontmatter for `agent`.
///
/// Operates on the rendered markdown so file-form and inline-form subagents
/// take the same path. `override_spec` replaces the frontmatter spec (and is
/// inserted when the document has frontmatter but no `model:` key). Without an
/// override, a document without frontmatter or without a `model:` key is
/// returned unchanged.
fn apply_model_to_frontmatter(
    content: &str,
    override_spec: Option<&str>,
    registry: Option<&crate::model::ProviderRegistry>,
    agent: Agent,
    default_provider: Option<&str>,
) -> String {
    let Some(rest) = content.strip_prefix("---\n") else {
        return content.to_string();
    };
    // Only the frontmatter block is rewritten; a `model:` line in the prompt
    // body is prose, not config.
    let Some(end) = rest.find("\n---") else {
        return content.to_string();
    };

    let mut out = String::with_capacity(content.len());
    out.push_str("---\n");
    let mut has_model = false;
    for line in rest[..end].split_inclusive('\n') {
        match line
            .strip_prefix("model:")
            .map(|v| (v.trim_end_matches('\n').trim(), line.ends_with('\n')))
        {
            Some((spec, newline)) if !spec.is_empty() || override_spec.is_some() => {
                has_model = true;
                let spec = override_spec.unwrap_or(spec);
                out.push_str("model: ");
                out.push_str(&render_model(spec, registry, agent, default_provider));
                if newline {
                    out.push('\n');
                }
            }
            _ => out.push_str(line),
        }
    }
    if !has_model {
        if let Some(spec) = override_spec {
            if !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("model: ");
            out.push_str(&render_model(spec, registry, agent, default_provider));
        }
    }
    out.push_str(&rest[end..]);
    out
}

/// Rewrite a Claude-style `tools: A, B` frontmatter line into OpenCode's
/// object form (`tools:\n  a: true\n  b: true`). OpenCode validates `tools` as
/// a `{name: bool}` map and rejects the comma-string, while Claude wants the
/// string — so the shared subagent `.md` needs converting on the way into
/// OpenCode's config dir. Only the inline comma-string form is touched; an
/// already-object `tools:` block (empty value, indented children) or an absent
/// line passes through unchanged.
///
/// ponytail: tool ids are mapped by lowercasing the Claude name (Read->read,
/// WebFetch->webfetch). Extend the map if a tool whose OpenCode id isn't the
/// lowercased Claude id shows up.
fn opencode_tools_frontmatter(content: &str) -> String {
    let Some(rest) = content.strip_prefix("---\n") else {
        return content.to_string();
    };
    let Some(end) = rest.find("\n---") else {
        return content.to_string();
    };

    let mut out = String::with_capacity(content.len());
    out.push_str("---\n");
    for line in rest[..end].split_inclusive('\n') {
        if let Some(spec) = line
            .strip_prefix("tools:")
            .map(|v| v.trim_end_matches('\n').trim())
            .filter(|s| !s.is_empty())
        {
            out.push_str("tools:\n");
            for name in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                out.push_str("  ");
                out.push_str(&name.to_lowercase());
                out.push_str(": true\n");
            }
            continue;
        }
        out.push_str(line);
    }
    out.push_str(&rest[end..]);
    out
}

/// Return the subagents directory for a given agent, or `None` if the agent
/// has no native subagent support.
pub fn subagents_dir(agent: Agent) -> Option<PathBuf> {
    let home = home::home_dir()?;
    match agent {
        Agent::Claude => {
            let base = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude"));
            Some(base.join("agents"))
        }
        Agent::OpenCode => Some(home.join(".config/opencode/agent")),
        Agent::Pi => {
            let pi_dir = if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
                PathBuf::from(dir)
            } else {
                home.join(".pi/agent")
            };
            Some(pi_dir.join("agents"))
        }
        // ponytail: omp/codex/copilot/gemini subagent dirs unverified;
        // add each here once its layout is confirmed.
        _ => None,
    }
}

/// Install the bootstrap subagents configured for `agent` into that agent's
/// subagents directory. Agents without one are a no-op. Overwrites without
/// prompting: the project config is the source of truth.
pub fn install_subagents_for_agent(
    agent: Agent,
    config: &BootstrapConfig,
    project_root: &Path,
    registry: Option<&crate::model::ProviderRegistry>,
    dry_run: bool,
) -> Result<Vec<SkillInstall>> {
    let defs = config.subagents_for(agent);
    if defs.is_empty() {
        return Ok(Vec::new());
    }
    let Some(base_dir) = subagents_dir(agent) else {
        return Ok(Vec::new());
    };
    let default_provider = config.default_provider_for(agent);
    let model_overrides = config
        .overrides_for(agent)
        .map(|o| &o.subagent_models)
        .cloned()
        .unwrap_or_default();
    install_subagents_into(
        &base_dir,
        &defs,
        project_root,
        registry,
        agent,
        default_provider,
        &model_overrides,
        dry_run,
    )
}

/// Directory-parameterized core of [`install_subagents_for_agent`], split out
/// so it can be tested without touching real agent config dirs.
fn install_subagents_into(
    base_dir: &Path,
    defs: &[SubagentDef],
    project_root: &Path,
    registry: Option<&crate::model::ProviderRegistry>,
    agent: Agent,
    default_provider: Option<&str>,
    model_overrides: &BTreeMap<String, String>,
    dry_run: bool,
) -> Result<Vec<SkillInstall>> {
    if !dry_run {
        fs::create_dir_all(base_dir)
            .with_context(|| format!("Failed to create {}", base_dir.display()))?;
    }

    let mut results = Vec::new();
    for def in defs {
        let (name, content) = def.resolve(project_root)?;
        let override_spec = model_overrides.get(&name).map(String::as_str);
        let content =
            apply_model_to_frontmatter(&content, override_spec, registry, agent, default_provider);
        let content = if matches!(agent, Agent::OpenCode) {
            opencode_tools_frontmatter(&content)
        } else {
            content
        };
        let dest = base_dir.join(format!("{name}.md"));

        let existing = fs::read_to_string(&dest).ok();
        results.push(match existing {
            Some(ref e) if *e == content => SkillInstall::UpToDate(name),
            existed => {
                if !dry_run {
                    fs::write(&dest, &content)
                        .with_context(|| format!("Failed to write {}", dest.display()))?;
                }
                if existed.is_some() {
                    SkillInstall::Updated(name)
                } else {
                    SkillInstall::Installed(name)
                }
            }
        });
    }
    Ok(results)
}

/// Derive the installed skill name from a source spec.
pub(crate) fn skill_name_from_source(source: &Source) -> Result<String> {
    let raw = match source {
        Source::LocalPath(p) => p.clone(),
        Source::Remote { url, .. } => url.clone(),
    };
    let trimmed = raw.trim_end_matches('/');
    let mut path = Path::new(trimmed);

    // `.../my-skill/SKILL.md` names the skill `my-skill`.
    if path.file_name().is_some_and(|f| f == "SKILL.md") {
        path = path.parent().unwrap_or(path);
    }

    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.trim_end_matches(".git").to_string())
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .ok_or_else(|| anyhow::anyhow!("Cannot derive a skill name from source: {raw}"))
}

/// Render a skill body as a minijinja template against the per-agent context
/// (`agent`, `agent_name`, plus configured `template_vars`), so one skill
/// source installs with concrete values baked in for each host agent.
/// Bodies without template delimiters pass through untouched, so ordinary
/// skills never hit the template engine (or its syntax rules) at all.
fn render_skill_template(
    raw: &str,
    agent: Agent,
    config: &BootstrapConfig,
    install_dir: Option<&Path>,
) -> Result<String> {
    if !raw.contains("{{") && !raw.contains("{%") {
        return Ok(raw.to_string());
    }
    let env = crate::template::create_template_env();
    let context = config.skill_template_context(agent, install_dir)?;
    crate::template::render_prompt_body(raw, &env, &context)
}

/// Render a hook command for one agent, resolving `{{ skill_install_dir }}`.
pub fn render_hook_command(
    command: &str,
    agent: Agent,
    config: &BootstrapConfig,
    install_dir: Option<&Path>,
) -> Result<String> {
    render_skill_template(command, agent, config, install_dir)
}

/// Up-to-date check for an installed skill: SKILL.md is compared against its
/// rendered form (dest holds the rendered copy, src the template); every
/// other file must match byte-for-byte.
fn skill_install_up_to_date(src: &Path, dest: &Path, rendered_skill_md: &str) -> Result<bool> {
    match fs::read_to_string(dest.join("SKILL.md")) {
        Ok(current) if current == rendered_skill_md => {}
        _ => return Ok(false),
    }
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        if entry.file_name() == "SKILL.md" {
            continue;
        }
        let dest_path = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            if !dest_path.is_dir() || !dirs_have_same_contents(&entry.path(), &dest_path)? {
                return Ok(false);
            }
        } else {
            let Ok(dest_bytes) = fs::read(&dest_path) else {
                return Ok(false);
            };
            if fs::read(entry.path())? != dest_bytes {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Compare two skill directories file-by-file so an unchanged skill is not
/// reported as an update on every `workmux setup`.
fn dirs_have_same_contents(src: &Path, dest: &Path) -> Result<bool> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let dest_path = dest.join(entry.file_name());
        let file_type = entry.file_type()?;

        if file_type.is_dir() {
            if !dest_path.is_dir() || !dirs_have_same_contents(&entry.path(), &dest_path)? {
                return Ok(false);
            }
        } else {
            let Ok(dest_bytes) = fs::read(&dest_path) else {
                return Ok(false);
            };
            if fs::read(entry.path())? != dest_bytes {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest).with_context(|| format!("Failed to create {}", dest.display()))?;

    for entry in fs::read_dir(src).with_context(|| format!("Failed to read {}", src.display()))? {
        let entry = entry?;
        let dest_path = dest.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), &dest_path)
                .with_context(|| format!("Failed to write {}", dest_path.display()))?;
        }
    }

    Ok(())
}

/// Print bootstrap configuration status.
pub fn print_bootstrap_info(config: &BootstrapConfig, agent: Agent) {
    let plugins = config.plugins_for(agent);
    if !plugins.is_empty() {
        println!("  Plugins: {}", style(plugins.join(", ")).dim());
    }

    let skills = config.skills_for(agent);
    if !skills.is_empty() {
        println!(
            "  Skills: {}",
            style(
                skills
                    .iter()
                    .map(|s| s.display())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .dim()
        );
    }

    let components = config.prompt_components_for(agent);
    if !components.is_empty() {
        println!(
            "  Prompt components: {}",
            style(components.join(", ")).dim()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_merge_component_by_name_and_path() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let dir = root.join(".workmux/prompt-components");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("terse.md"), "Be concise.\n").unwrap();
        fs::write(root.join("outside.md"), "Outside rule.\n").unwrap();

        let components = vec![
            "terse".to_string(),
            root.join("outside.md").to_string_lossy().into_owned(),
            "./outside.md".to_string(),
            "missing".to_string(),
        ];
        let (merged, added) = merge_prompt_components("", &components, root).unwrap();
        assert!(added);
        assert!(merged.contains("Be concise."));
        // Same file via two spellings is merged once; a missing name is skipped.
        assert_eq!(merged.matches("Outside rule.").count(), 1);
    }

    #[test]
    fn test_source_git_url_detection() {
        let github_https = Source::Remote {
            url: "https://github.com/user/repo".to_string(),
            r#ref: None,
        };
        assert!(github_https.is_git_url());

        let github_ssh = Source::Remote {
            url: "git@github.com:user/repo".to_string(),
            r#ref: None,
        };
        assert!(github_ssh.is_git_url());

        let git_url = Source::Remote {
            url: "https://example.com/repo.git".to_string(),
            r#ref: None,
        };
        assert!(git_url.is_git_url());

        let raw_url = Source::Remote {
            url: "https://example.com/skill.tar.gz".to_string(),
            r#ref: None,
        };
        assert!(!raw_url.is_git_url());

        let local = Source::LocalPath("./skills/my-skill".to_string());
        assert!(!local.is_git_url());
    }

    #[test]
    fn test_plugins_for_agent_without_overrides() {
        let config = BootstrapConfig {
            default_plugins: vec!["caveman".to_string(), "opencode".to_string()],
            ..Default::default()
        };

        let plugins = config.plugins_for(Agent::Claude);
        assert_eq!(plugins, vec!["caveman", "opencode"]);
    }

    #[test]
    fn test_plugins_for_agent_with_overrides() {
        let mut overrides = BTreeMap::new();
        overrides.insert(
            "claude code".to_string(),
            AgentBootstrapOverrides {
                additional_plugins: vec!["custom-plugin".to_string()],
                ..Default::default()
            },
        );

        let config = BootstrapConfig {
            default_plugins: vec!["caveman".to_string()],
            agents: overrides,
            ..Default::default()
        };

        let plugins = config.plugins_for(Agent::Claude);
        assert_eq!(plugins.len(), 2);
        assert!(plugins.contains(&"caveman".to_string()));
        assert!(plugins.contains(&"custom-plugin".to_string()));
    }

    #[test]
    fn test_skills_with_mixed_sources() {
        let config = BootstrapConfig {
            default_skills: vec![
                Source::LocalPath("./shared/skill1".to_string()).into(),
                Source::Remote {
                    url: "https://github.com/user/skill-repo".to_string(),
                    r#ref: None,
                }
                .into(),
            ],
            ..Default::default()
        };

        let skills = config.skills_for(Agent::Claude);
        assert_eq!(skills.len(), 2);
    }

    #[test]
    fn test_prompt_components_for_agent_default() {
        let config = BootstrapConfig {
            default_prompt_components: vec!["caveman".to_string(), "code-review".to_string()],
            ..Default::default()
        };

        let components = config.prompt_components_for(Agent::Claude);
        assert_eq!(components, vec!["caveman", "code-review"]);
    }

    #[test]
    fn test_prompt_components_for_agent_with_additional() {
        let mut overrides = BTreeMap::new();
        overrides.insert(
            "claude code".to_string(),
            AgentBootstrapOverrides {
                additional_prompt_components: vec!["claude-optimization".to_string()],
                ..Default::default()
            },
        );

        let config = BootstrapConfig {
            default_prompt_components: vec!["caveman".to_string()],
            agents: overrides,
            ..Default::default()
        };

        let components = config.prompt_components_for(Agent::Claude);
        assert_eq!(components.len(), 2);
        assert!(components.contains(&"caveman".to_string()));
        assert!(components.contains(&"claude-optimization".to_string()));
    }

    fn ponytail_feature() -> BTreeMap<String, FeatureConfig> {
        let mut agent_plugins = BTreeMap::new();
        agent_plugins.insert(
            "pi".to_string(),
            "git:github.com/DietrichGebert/ponytail".to_string(),
        );
        let mut features = BTreeMap::new();
        features.insert(
            "ponytail".to_string(),
            FeatureConfig {
                default: Some("ponytail".to_string()),
                agent_plugins,
            },
        );
        features
    }

    #[test]
    fn test_feature_resolves_to_plugin_for_matching_agent() {
        let config = BootstrapConfig {
            features: ponytail_feature(),
            ..Default::default()
        };
        // pi has a dedicated plugin impl -> plugin, no prompt fallback.
        assert_eq!(
            config.plugins_for(Agent::Pi),
            vec!["git:github.com/DietrichGebert/ponytail".to_string()]
        );
        assert!(config.prompt_components_for(Agent::Pi).is_empty());
    }

    #[test]
    fn test_feature_falls_back_to_default_prompt_for_other_agents() {
        let config = BootstrapConfig {
            features: ponytail_feature(),
            ..Default::default()
        };
        // Claude has no plugin impl -> falls back to the default prompt component.
        assert!(config.plugins_for(Agent::Claude).is_empty());
        assert_eq!(
            config.prompt_components_for(Agent::Claude),
            vec!["ponytail".to_string()]
        );
    }

    #[test]
    fn test_feature_default_can_be_disabled_per_agent() {
        let mut overrides = BTreeMap::new();
        overrides.insert(
            "claude code".to_string(),
            AgentBootstrapOverrides {
                disabled_prompt_components: vec!["ponytail".to_string()],
                ..Default::default()
            },
        );
        let config = BootstrapConfig {
            features: ponytail_feature(),
            agents: overrides,
            ..Default::default()
        };
        assert!(config.prompt_components_for(Agent::Claude).is_empty());
    }

    #[test]
    fn test_feature_agent_key_accepts_display_name() {
        let mut agent_plugins = BTreeMap::new();
        agent_plugins.insert("claude code".to_string(), "npm:some-plugin".to_string());
        let mut features = BTreeMap::new();
        features.insert(
            "thing".to_string(),
            FeatureConfig {
                default: Some("thing-prompt".to_string()),
                agent_plugins,
            },
        );
        let config = BootstrapConfig {
            features,
            ..Default::default()
        };
        assert_eq!(
            config.plugins_for(Agent::Claude),
            vec!["npm:some-plugin".to_string()]
        );
    }

    #[test]
    fn test_skill_name_from_source() {
        let name = |s: &str| skill_name_from_source(&Source::LocalPath(s.to_string())).unwrap();
        assert_eq!(name("./skills/workmux"), "workmux");
        assert_eq!(name("./skills/workmux/"), "workmux");
        assert_eq!(name("./skills/workmux/SKILL.md"), "workmux");
        assert_eq!(name("/abs/path/my-skill"), "my-skill");

        let remote = Source::Remote {
            url: "https://github.com/user/cool-skill.git".to_string(),
            r#ref: None,
        };
        assert_eq!(skill_name_from_source(&remote).unwrap(), "cool-skill");
    }

    #[test]
    fn test_copy_dir_recursive_and_compare() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src/my-skill");
        fs::create_dir_all(src.join("references")).unwrap();
        fs::write(src.join("SKILL.md"), "---\nname: my-skill\n---\nbody").unwrap();
        fs::write(src.join("references/extra.md"), "extra").unwrap();

        let dest = tmp.path().join("dest/my-skill");
        copy_dir_recursive(&src, &dest).unwrap();

        assert_eq!(
            fs::read_to_string(dest.join("SKILL.md")).unwrap(),
            "---\nname: my-skill\n---\nbody"
        );
        assert_eq!(
            fs::read_to_string(dest.join("references/extra.md")).unwrap(),
            "extra"
        );
        assert!(dirs_have_same_contents(&src, &dest).unwrap());

        // A changed nested file must be detected.
        fs::write(src.join("references/extra.md"), "changed").unwrap();
        assert!(!dirs_have_same_contents(&src, &dest).unwrap());

        // Re-copying makes them equal again.
        copy_dir_recursive(&src, &dest).unwrap();
        assert!(dirs_have_same_contents(&src, &dest).unwrap());
    }

    #[test]
    fn test_dirs_have_same_contents_missing_dest_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dest = tmp.path().join("dest");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(src.join("SKILL.md"), "a").unwrap();

        assert!(!dirs_have_same_contents(&src, &dest).unwrap());
    }

    #[test]
    fn settings_patch_is_per_agent() {
        let cfg: BootstrapConfig = serde_yaml::from_str(
            r#"
agents:
  pi:
    settings:
      defaultTools: [read, edit]
      taskflow:
        piChild:
          resourceProfile: allowlist
      theme: null
"#,
        )
        .unwrap();

        let patch = cfg.settings_for(Agent::Pi).expect("pi patch");
        assert_eq!(patch["defaultTools"], serde_json::json!(["read", "edit"]));
        assert_eq!(patch["taskflow"]["piChild"]["resourceProfile"], "allowlist");
        assert!(patch["theme"].is_null(), "null survives deserialization as a delete marker");
        assert!(cfg.settings_for(Agent::Claude).is_none());
    }

    // PI_CODING_AGENT_DIR is process-global; serialize the tests that set it.
    static PI_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_subagents_dir_pi_with_env_var() {
        let _guard = PI_DIR_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("PI_CODING_AGENT_DIR");
        unsafe {
            std::env::set_var("PI_CODING_AGENT_DIR", "/tmp/custom-pi-dir");
        }

        let dir = subagents_dir(Agent::Pi);

        unsafe {
            match prev {
                Some(v) => std::env::set_var("PI_CODING_AGENT_DIR", v),
                None => std::env::remove_var("PI_CODING_AGENT_DIR"),
            }
        }

        assert_eq!(dir, Some(PathBuf::from("/tmp/custom-pi-dir/agents")));
    }

    #[test]
    fn test_subagents_dir_pi_without_env_var() {
        let _guard = PI_DIR_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("PI_CODING_AGENT_DIR");
        unsafe {
            std::env::remove_var("PI_CODING_AGENT_DIR");
        }

        let dir = subagents_dir(Agent::Pi);

        unsafe {
            if let Some(v) = prev {
                std::env::set_var("PI_CODING_AGENT_DIR", v);
            }
        }

        assert_eq!(dir, Some(home::home_dir().unwrap().join(".pi/agent/agents")));
    }

    #[test]
    fn test_install_skills_for_agent_without_skills_dir_is_noop() {
        let config = BootstrapConfig {
            default_skills: vec![Source::LocalPath("./skills/workmux".to_string()).into()],
            ..Default::default()
        };
        // Codex has no skills directory, so nothing is installed and the
        // (possibly nonexistent) source path is never touched.
        let results =
            install_skills_for_agent(Agent::Codex, &config, Path::new("/nonexistent"), false).unwrap();
        assert!(results.is_empty());
    }

    /// Serializes the tests that redirect `OMP_CODING_AGENT_DIR`, so they
    /// cannot clobber each other's saved value when run in parallel.
    static OMP_DIR_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn test_install_skills_for_agent_end_to_end() {
        let _guard = OMP_DIR_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("project");
        let skill_src = project_root.join("skills/workmux");
        fs::create_dir_all(&skill_src).unwrap();
        fs::write(skill_src.join("SKILL.md"), "---\nname: workmux\n---\nv1").unwrap();

        let agent_dir = tmp.path().join("omp-agent");
        // SAFETY: OMP_CODING_AGENT_DIR is read by no other test in this crate,
        // and OMP_DIR_ENV_LOCK serializes the tests here that write it, so
        // this cannot race a concurrently running test. The original value is
        // restored below.
        let prev = std::env::var_os("OMP_CODING_AGENT_DIR");
        unsafe {
            std::env::set_var("OMP_CODING_AGENT_DIR", &agent_dir);
        }

        let config = BootstrapConfig {
            default_skills: vec![Source::LocalPath("./skills/workmux".to_string()).into()],
            ..Default::default()
        };

        let installed = agent_dir.join("skills/workmux/SKILL.md");

        let first = install_skills_for_agent(Agent::Omp, &config, &project_root, false).unwrap();
        let first_ok = first == vec![SkillInstall::Installed("workmux".to_string())]
            && fs::read_to_string(&installed).unwrap_or_default() == "---\nname: workmux\n---\nv1";

        // Re-running is idempotent.
        let second = install_skills_for_agent(Agent::Omp, &config, &project_root, false).unwrap();
        let second_ok = second == vec![SkillInstall::UpToDate("workmux".to_string())];

        // Editing the source re-installs it.
        fs::write(skill_src.join("SKILL.md"), "---\nname: workmux\n---\nv2").unwrap();
        let third = install_skills_for_agent(Agent::Omp, &config, &project_root, false).unwrap();
        let third_ok = third == vec![SkillInstall::Updated("workmux".to_string())]
            && fs::read_to_string(&installed).unwrap_or_default() == "---\nname: workmux\n---\nv2";

        unsafe {
            match prev {
                Some(v) => std::env::set_var("OMP_CODING_AGENT_DIR", v),
                None => std::env::remove_var("OMP_CODING_AGENT_DIR"),
            }
        }

        assert!(first_ok, "first install: {first:?}");
        assert!(second_ok, "second install should be a no-op: {second:?}");
        assert!(third_ok, "changed source should update: {third:?}");
    }

    #[test]
    fn test_render_skill_template_resolves_builtins_and_vars() {
        let config = BootstrapConfig {
            template_vars: BTreeMap::from([
                (
                    "review_cmd".to_string(),
                    TemplateVar::PerAgent(BTreeMap::from([
                        ("claude".to_string(), "/code-review".to_string()),
                        ("default".to_string(), "/review".to_string()),
                    ])),
                ),
                (
                    "orchestrator".to_string(),
                    TemplateVar::Scalar("orca".to_string()),
                ),
            ]),
            ..Default::default()
        };

        let raw = "start --agent {{ agent }} ({{ agent_name }}) via {{ orchestrator }}: {{ review_cmd }}";
        let claude = render_skill_template(raw, Agent::Claude, &config, None).unwrap();
        assert_eq!(claude, "start --agent claude (Claude Code) via orca: /code-review");
        let omp = render_skill_template(raw, Agent::Omp, &config, None).unwrap();
        assert_eq!(omp, "start --agent omp (omp) via orca: /review");

        // No delimiters -> engine bypassed, literal braces survive.
        let plain = "json body { \"a\": 1 }";
        assert_eq!(render_skill_template(plain, Agent::Claude, &config, None).unwrap(), plain);

        // Built-ins cannot be shadowed.
        let bad = BootstrapConfig {
            template_vars: BTreeMap::from([(
                "agent".to_string(),
                TemplateVar::Scalar("nope".to_string()),
            )]),
            ..Default::default()
        };
        assert!(render_skill_template("{{ agent }}", Agent::Claude, &bad, None).is_err());
    }

    #[test]
    fn test_install_skills_for_agent_reports_remote_as_skipped() {
        let _guard = OMP_DIR_ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("omp-agent");
        // SAFETY: see test_install_skills_for_agent_end_to_end.
        let prev = std::env::var_os("OMP_CODING_AGENT_DIR");
        unsafe {
            std::env::set_var("OMP_CODING_AGENT_DIR", &agent_dir);
        }

        let config = BootstrapConfig {
            default_skills: vec![
                Source::Remote {
                    url: "https://github.com/user/cool-skill".to_string(),
                    r#ref: None,
                }
                .into(),
            ],
            ..Default::default()
        };
        let results = install_skills_for_agent(Agent::Omp, &config, tmp.path(), false);

        unsafe {
            match prev {
                Some(v) => std::env::set_var("OMP_CODING_AGENT_DIR", v),
                None => std::env::remove_var("OMP_CODING_AGENT_DIR"),
            }
        }

        let results = results.unwrap();
        assert!(
            matches!(results.as_slice(), [SkillInstall::Skipped(name, _)] if name == "cool-skill"),
            "{results:?}"
        );
    }

    #[test]
    fn test_default_skills_parses_from_yaml() {
        let yaml = "default_skills:\n  - ./skills/workmux\n  - url: https://github.com/u/r\n    ref: main\n";
        let config: BootstrapConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.default_skills.len(), 2);
        assert!(matches!(config.default_skills[0].source, Source::LocalPath(_)));
        assert!(config.default_skills[1].source.is_git_url());
    }

    #[test]
    fn test_subagents_parse_from_yaml_file_and_inline() {
        let yaml = "default_subagents:\n  - ./agents/reviewer.md\n  - name: planner\n    description: 'Plans: work'\n    prompt: |\n      You plan.\n";
        let config: BootstrapConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.default_subagents.len(), 2);
        assert!(matches!(config.default_subagents[0], SubagentDef::File(_)));
        assert!(matches!(
            config.default_subagents[1],
            SubagentDef::Inline { .. }
        ));
    }

    #[test]
    fn test_subagent_inline_resolve_renders_frontmatter() {
        let def = SubagentDef::Inline {
            name: "planner".to_string(),
            description: Some("Plans: work".to_string()),
            prompt: "You plan.".to_string(),
            tools: vec!["Read".to_string(), "Grep".to_string()],
            model: Some("sonnet".to_string()),
        };
        let (name, content) = def.resolve(Path::new("/nonexistent")).unwrap();
        assert_eq!(name, "planner");
        assert!(content.starts_with("---\n"), "{content}");
        assert!(content.contains("name: planner"), "{content}");
        assert!(content.contains("'Plans: work'"), "{content}");
        assert!(content.contains("tools: Read, Grep"), "{content}");
        assert!(content.contains("model: sonnet"), "{content}");
        assert!(content.ends_with("---\n\nYou plan."), "{content}");
    }

    #[test]
    fn test_opencode_tools_frontmatter_converts_string_to_object() {
        let claude = "---\nname: reviewer\ntools: Read, Grep, Glob, Bash\nmodel: opus\n---\nReview.";
        let out = opencode_tools_frontmatter(claude);
        assert_eq!(
            out,
            "---\nname: reviewer\ntools:\n  read: true\n  grep: true\n  glob: true\n  bash: true\nmodel: opus\n---\nReview."
        );

        // No tools line, or a body-only doc: passed through untouched.
        let no_tools = "---\nname: reviewer\nmodel: opus\n---\nReview.";
        assert_eq!(opencode_tools_frontmatter(no_tools), no_tools);
        let no_fm = "Just a body, no frontmatter.";
        assert_eq!(opencode_tools_frontmatter(no_fm), no_fm);
    }

    #[test]
    fn test_install_subagents_into_end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        let project_root = tmp.path().join("project");
        fs::create_dir_all(project_root.join("agents")).unwrap();
        fs::write(
            project_root.join("agents/reviewer.md"),
            "---\nname: reviewer\n---\nReview code.",
        )
        .unwrap();

        let defs = vec![
            SubagentDef::File("./agents/reviewer.md".to_string()),
            SubagentDef::Inline {
                name: "planner".to_string(),
                description: None,
                prompt: "You plan.".to_string(),
                tools: Vec::new(),
                model: None,
            },
        ];

        let dest = tmp.path().join("agent-home/agents");
        let first = install_subagents_into(&dest, &defs, &project_root, None, Agent::Claude, None, &BTreeMap::new(), false).unwrap();
        assert_eq!(
            first,
            vec![
                SkillInstall::Installed("reviewer".to_string()),
                SkillInstall::Installed("planner".to_string()),
            ]
        );
        assert_eq!(
            fs::read_to_string(dest.join("reviewer.md")).unwrap(),
            "---\nname: reviewer\n---\nReview code."
        );
        assert!(fs::read_to_string(dest.join("planner.md"))
            .unwrap()
            .contains("You plan."));

        // Re-running is idempotent.
        let second = install_subagents_into(&dest, &defs, &project_root, None, Agent::Claude, None, &BTreeMap::new(), false).unwrap();
        assert_eq!(
            second,
            vec![
                SkillInstall::UpToDate("reviewer".to_string()),
                SkillInstall::UpToDate("planner".to_string()),
            ]
        );

        // Editing the source updates the install.
        fs::write(
            project_root.join("agents/reviewer.md"),
            "---\nname: reviewer\n---\nReview harder.",
        )
        .unwrap();
        let third = install_subagents_into(&dest, &defs, &project_root, None, Agent::Claude, None, &BTreeMap::new(), false).unwrap();
        assert_eq!(third[0], SkillInstall::Updated("reviewer".to_string()));
    }

    fn model_registry() -> crate::model::ProviderRegistry {
        use crate::model::{ProviderConfig, ProviderModel};
        let model = |name: &str, id: &str| ProviderModel {
            name: name.into(),
            id: id.into(),
            tier: None,
            limit: None,
            compaction_limit: None,
        };
        let mut reg = crate::model::ProviderRegistry::new();
        reg.insert(
            "anthropic".into(),
            ProviderConfig {
                limit: Some(200_000),
                compaction_limit: None,
                models: vec![
                    model("haiku", "claude-haiku-4-5"),
                    model("opus", "claude-opus-4-8"),
                ],
                ..Default::default()
            },
        );
        reg.insert(
            "bedrock".into(),
            ProviderConfig {
                limit: Some(200_000),
                compaction_limit: None,
                models: vec![model("opus", "anthropic.claude-opus-4-8")],
                ..Default::default()
            },
        );
        reg
    }

    #[test]
    fn test_render_model_per_host_and_provider() {
        let reg = model_registry();
        let r = |spec: &str, agent| render_model(spec, Some(&reg), agent, None);

        // Unique logical name resolves; host decides the rendered form.
        assert_eq!(r("haiku", Agent::Claude), "claude-haiku-4-5");
        assert_eq!(r("haiku", Agent::OpenCode), "anthropic/claude-haiku-4-5");

        // Ambiguous across providers -> pass through, never guess.
        assert_eq!(r("opus", Agent::Claude), "opus");
        // ...unless the spec pins the provider.
        assert_eq!(r("bedrock/opus", Agent::Claude), "anthropic.claude-opus-4-8");
        assert_eq!(r("anthropic/opus", Agent::Claude), "claude-opus-4-8");

        // Unknown names and an absent registry pass through untouched, so a
        // shared config works on a machine that declares no providers.
        assert_eq!(r("gpt-5", Agent::Claude), "gpt-5");
        assert_eq!(render_model("haiku", None, Agent::OpenCode, None), "haiku");
    }

    /// The personal-vs-work case: one shared `model: opus`, each agent pinned
    /// to its own provider, no per-subagent qualification.
    #[test]
    fn test_default_provider_per_agent() {
        let reg = model_registry();
        let yaml = "agents:\n  claude:\n    default_provider: anthropic\n  opencode:\n    default_provider: bedrock\n";
        let config: BootstrapConfig = serde_yaml::from_str(yaml).unwrap();

        // Short-id keys (`claude`) must resolve, not just display names.
        assert_eq!(config.default_provider_for(Agent::Claude), Some("anthropic"));
        assert_eq!(config.default_provider_for(Agent::OpenCode), Some("bedrock"));
        assert_eq!(config.default_provider_for(Agent::Pi), None);

        // `opus` is ambiguous registry-wide, but each agent's default picks one.
        let render = |agent| {
            render_model(
                "opus",
                Some(&reg),
                agent,
                config.default_provider_for(agent),
            )
        };
        assert_eq!(render(Agent::Claude), "claude-opus-4-8");
        assert_eq!(render(Agent::OpenCode), "bedrock/anthropic.claude-opus-4-8");
        // Pi maps the bare tier alias to its taskflow role placeholder
        // before registry lookup, regardless of provider ambiguity.
        assert_eq!(render(Agent::Pi), "\"{{expert}}\"");

        // An explicit prefix in the spec outranks the agent default.
        assert_eq!(
            render_model("bedrock/opus", Some(&reg), Agent::Claude, Some("anthropic")),
            "anthropic.claude-opus-4-8"
        );
        // A default that doesn't serve the model passes through rather than
        // silently falling back to another provider.
        assert_eq!(
            render_model("haiku", Some(&reg), Agent::Claude, Some("bedrock")),
            "haiku"
        );
    }

    #[test]
    fn test_render_model_pi_tier_aliases() {
        let reg = model_registry();

        // Bare tier aliases map to taskflow role placeholders, independent
        // of whether/how the registry resolves them.
        assert_eq!(
            render_model("haiku", Some(&reg), Agent::Pi, None),
            "\"{{scout}}\""
        );
        assert_eq!(
            render_model("sonnet", Some(&reg), Agent::Pi, None),
            "\"{{builder}}\""
        );
        assert_eq!(
            render_model("opus", Some(&reg), Agent::Pi, None),
            "\"{{expert}}\""
        );
        assert_eq!(
            render_model("haiku", None, Agent::Pi, None),
            "\"{{scout}}\""
        );

        // An explicit provider id is not one of the aliases -> unchanged.
        assert_eq!(
            render_model("anthropic.claude-opus-4-8", Some(&reg), Agent::Pi, None),
            "anthropic.claude-opus-4-8"
        );

        // A `provider/id` spec is also left alone (no alias match).
        assert_eq!(
            render_model("bedrock/opus", Some(&reg), Agent::Pi, None),
            "anthropic.claude-opus-4-8"
        );
    }

    #[test]
    fn test_theme_for_agent() {
        // Scalar form: same theme everywhere.
        let config: BootstrapConfig = serde_yaml::from_str("theme: catppuccin").unwrap();
        assert_eq!(config.theme_for(Agent::OpenCode), Some("catppuccin"));
        assert_eq!(config.theme_for(Agent::Codex), Some("catppuccin"));

        // Map form: per-agent name (theme names aren't portable), with fallback.
        let yaml = "theme:\n  claude code: dark\n  default: catppuccin\n";
        let config: BootstrapConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.theme_for(Agent::Claude), Some("dark"));
        assert_eq!(config.theme_for(Agent::OpenCode), Some("catppuccin"));

        // No `default` -> agents without an entry keep their own theme.
        let config: BootstrapConfig = serde_yaml::from_str("theme:\n  opencode: nord\n").unwrap();
        assert_eq!(config.theme_for(Agent::OpenCode), Some("nord"));
        assert_eq!(config.theme_for(Agent::Claude), None);

        // Unset -> never touch any agent's config.
        let config = BootstrapConfig::default();
        assert_eq!(config.theme_for(Agent::OpenCode), None);
    }

    #[test]
    fn test_apply_model_to_frontmatter() {
        let reg = model_registry();
        let doc = "---\nname: explore\nmodel: haiku\n---\n\nBody mentions model: haiku too.";

        let out = apply_model_to_frontmatter(doc, None, Some(&reg), Agent::OpenCode, None);
        assert!(out.contains("model: anthropic/claude-haiku-4-5"), "{out}");
        assert!(out.contains("name: explore"), "{out}");
        // The body is prose, not config — it must survive untouched.
        assert!(out.ends_with("Body mentions model: haiku too."), "{out}");

        // No frontmatter, or no registry, leaves the document alone.
        assert_eq!(
            apply_model_to_frontmatter("plain", None, Some(&reg), Agent::Claude, None),
            "plain"
        );
        assert_eq!(apply_model_to_frontmatter(doc, None, None, Agent::Claude, None), doc);

        // A per-agent override replaces the spec and passes an unregistered
        // provider-qualified spec through verbatim.
        let out = apply_model_to_frontmatter(
            doc,
            Some("corp/claude-haiku-4-5"),
            Some(&reg),
            Agent::OpenCode,
            None,
        );
        assert!(out.contains("model: corp/claude-haiku-4-5"), "{out}");

        // An override is inserted when the frontmatter has no model key.
        let no_model = "---\nname: explore\n---\n\nBody.";
        let out = apply_model_to_frontmatter(
            no_model,
            Some("corp/claude-haiku-4-5"),
            Some(&reg),
            Agent::OpenCode,
            None,
        );
        assert!(out.contains("name: explore\nmodel: corp/claude-haiku-4-5\n---"), "{out}");
    }

    #[test]
    fn test_subagents_for_agent_with_overrides() {
        let mut overrides = BTreeMap::new();
        overrides.insert(
            "claude code".to_string(),
            AgentBootstrapOverrides {
                additional_subagents: vec![SubagentDef::File("./agents/extra.md".to_string())],
                ..Default::default()
            },
        );
        let config = BootstrapConfig {
            default_subagents: vec![SubagentDef::File("./agents/reviewer.md".to_string())],
            agents: overrides,
            ..Default::default()
        };
        assert_eq!(config.subagents_for(Agent::Claude).len(), 2);
        assert_eq!(config.subagents_for(Agent::Pi).len(), 1);
    }

    

    #[test]
    fn test_feature_config_flatten_roundtrip() {
        let yaml = "default: ponytail\npi: 'git:github.com/DietrichGebert/ponytail'\nomp: pi-ponytail@marketplace\n";
        let feature: FeatureConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(feature.default.as_deref(), Some("ponytail"));
        assert_eq!(
            feature.plugin_for(Agent::Pi),
            Some("git:github.com/DietrichGebert/ponytail")
        );
        assert_eq!(feature.plugin_for(Agent::Omp), Some("pi-ponytail@marketplace"));
        assert_eq!(feature.plugin_for(Agent::Claude), None);
    }
}

#[cfg(test)]
mod skill_install_dir_tests {
    use super::*;

    #[test]
    fn skill_install_dir_renders_to_the_destination() {
        let config = BootstrapConfig::default();
        let got = render_skill_template(
            "run {{ skill_install_dir }}/scripts/x.sh",
            Agent::Claude,
            &config,
            Some(Path::new("/home/u/.claude/skills/auto-git")),
        )
        .unwrap();
        assert_eq!(got, "run /home/u/.claude/skills/auto-git/scripts/x.sh");
    }

    #[test]
    fn hook_command_renders_through_the_same_context() {
        let config = BootstrapConfig::default();
        let got = render_hook_command(
            "bash \"{{ skill_install_dir }}/scripts/autocommit.sh\"",
            Agent::Claude,
            &config,
            Some(Path::new("/dest/auto-git")),
        )
        .unwrap();
        assert_eq!(got, "bash \"/dest/auto-git/scripts/autocommit.sh\"");
    }

    /// The variable is reserved: a user template_var must not shadow it.
    #[test]
    fn skill_install_dir_cannot_be_overridden() {
        let mut config = BootstrapConfig::default();
        config.template_vars.insert(
            "skill_install_dir".to_string(),
            TemplateVar::Scalar("evil".to_string()),
        );
        assert!(config
            .skill_template_context(Agent::Claude, Some(Path::new("/real")))
            .is_err());
    }

    /// An installed SKILL.md referencing the variable contains the per-agent
    /// absolute path, sitting next to the copied script it names.
    #[test]
    fn installed_skill_md_contains_the_absolute_path() {
        let tmp = std::env::temp_dir().join(format!(
            "workmux-sid-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&tmp);
        let src = tmp.join("src-skill");
        fs::create_dir_all(src.join("scripts")).unwrap();
        fs::write(
            src.join("SKILL.md"),
            "Run {{ skill_install_dir }}/scripts/go.sh",
        )
        .unwrap();
        fs::write(src.join("scripts/go.sh"), "#!/bin/sh\n").unwrap();

        let config = BootstrapConfig {
            default_skills: vec![Source::LocalPath(src.display().to_string()).into()],
            ..Default::default()
        };

        // Install into a scratch Claude config dir.
        let dest_base = tmp.join("claude/skills");
        let prev = std::env::var("CLAUDE_CONFIG_DIR").ok();
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", tmp.join("claude")) };
        let results = install_skills_for_agent(Agent::Claude, &config, &tmp, false).unwrap();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }

        assert!(matches!(results[0], SkillInstall::Installed(_)), "{results:?}");
        let installed = fs::read_to_string(dest_base.join("src-skill/SKILL.md")).unwrap();
        assert_eq!(
            installed,
            format!("Run {}/scripts/go.sh", dest_base.join("src-skill").display())
        );
        let _ = fs::remove_dir_all(&tmp);
    }
}
