//! Named agent capability profiles — the configurable definition layer.
//!
//! An `AgentDefinition` is a reusable bundle: system-prompt template, model,
//! permission mode, and bootstrap overrides. It binds to a pipeline node via
//! `agent_ref: "name"` in the harness YAML. Orthogonal to orchestration — it
//! describes HOW a node behaves, not WHEN it runs (that stays in the DAG).
//!
//! RED LINE: agent definitions are pure configuration. They must never become
//! an orchestration mechanism. No "agent decides to call another agent" — all
//! transitions go through the graph.

use crate::bootstrap::Source;
use crate::config::McpServerConfig;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A named, reusable agent capability profile.
///
/// Bound to a graph node via `agent_ref: "name"` in the harness YAML.
/// Provides default `permission_mode`, a `prompt_template` prefix, and
/// bootstrap overrides (plugins/skills/prompt_components/MCP) that are merged
/// on top of the project-level `bootstrap:` config.
///
/// The node's own `permission_mode` takes precedence over the definition's
/// default; similarly, node-level `input_variables` fill the template holes.
///
/// ```yaml
/// # .muxix.yaml
/// agent_defs:
///   planner:
///     description: "Read-only planning agent"
///     agent_type: claude
///     permission_mode: plan
///     model: claude-sonnet-4-6
///     prompt_template: |
///       You are a careful planner. Think step-by-step before any code.
///       Context: {{CONTEXT}}
///
///   implementer:
///     description: "Full-access implementation agent"
///     prompt_ref: default-implementer   # from .muxix/prompts/ or prompt_defs
///     bootstrap:
///       prompt_components: [caveman-full]
///       mcp:
///         socraticode:
///           command: npx
///           args: ["-y", "socraticode"]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentDefinition {
    /// Human description of this agent's role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Which base agent CLI to invoke (claude, gemini, codex, …).
    /// If unset, inherits from the project's `agent:` config.
    /// Note: per-node agent switching requires agent-model-unification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,

    /// Model override (e.g. "claude-sonnet-4-6"). Agent-type-specific.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    /// Default permission mode for nodes using this definition (e.g. "plan").
    /// Overridable per-node via the node's own `permission_mode:`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<String>,

    /// Inline system-prompt template prepended to the node's own prompt.
    /// Supports `{{VARIABLE}}` holes filled from `input_variables` + task vars.
    /// Mutually exclusive with `prompt_ref`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_template: Option<String>,

    /// Reference to a named prompt in the prompt registry (`.muxix/prompts/`
    /// or `prompt_defs:` in config). Alternative to inline `prompt_template`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_ref: Option<String>,

    /// Bootstrap overrides merged on top of the project bootstrap when this
    /// definition is used by a node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<AgentDefinitionBootstrap>,
}

impl AgentDefinition {
    /// Returns a human-readable summary of what capabilities/permissions this
    /// definition grants. Used for trust review before adopting a remote profile.
    pub fn trust_summary(&self) -> TrustSummary {
        let mut elevated = false;
        let mut items: Vec<String> = Vec::new();

        if let Some(ref mode) = self.permission_mode {
            if mode != "plan" {
                elevated = true;
                items.push(format!("permission_mode: {mode}"));
            }
        }

        if let Some(ref bootstrap) = self.bootstrap {
            if !bootstrap.mcp.is_empty() {
                let names: Vec<&str> = bootstrap.mcp.keys().map(|s| s.as_str()).collect();
                elevated = true;
                items.push(format!("mcp servers: {}", names.join(", ")));
            }
            if !bootstrap.plugins.is_empty() {
                items.push(format!("plugins: {}", bootstrap.plugins.join(", ")));
            }
            if !bootstrap.skills.is_empty() {
                let names: Vec<String> = bootstrap.skills.iter().map(|s| s.display()).collect();
                items.push(format!("skills: {}", names.join(", ")));
            }
        }

        TrustSummary { elevated, items }
    }

    /// Resolve the effective prompt template content given a prompt registry.
    /// Returns the inline `prompt_template` if set, otherwise looks up `prompt_ref`.
    pub fn resolve_template<'a>(
        &'a self,
        prompt_defs: &'a BTreeMap<String, crate::prompt::PromptTemplate>,
    ) -> Option<&'a str> {
        if let Some(ref tmpl) = self.prompt_template {
            return Some(tmpl.as_str());
        }
        if let Some(ref ref_name) = self.prompt_ref {
            return prompt_defs.get(ref_name).map(|pt| pt.content.as_str());
        }
        None
    }
}

/// Bootstrap overrides carried by an agent definition.
/// Merged on top of the project's `bootstrap:` section at node execution time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentDefinitionBootstrap {
    /// Extra Claude Code plugins (marketplace IDs) to activate for this agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<String>,

    /// Extra skills (local paths or git remotes) to install for this agent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<Source>,

    /// Extra prompt components (from `.muxix/prompt-components/`) to inject.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_components: Vec<String>,

    /// Extra MCP servers to activate when this definition is used.
    /// Trust note: MCP servers provide tools to the agent — review before importing.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub mcp: BTreeMap<String, McpServerConfig>,
}

/// Human-readable summary of the capabilities and elevated permissions a
/// definition grants. Shown before adopting a remote profile.
#[derive(Debug, Clone)]
pub struct TrustSummary {
    /// True if the definition grants any elevated permissions (non-plan mode, MCP).
    pub elevated: bool,
    /// Bullet items describing what the definition grants.
    pub items: Vec<String>,
}

impl TrustSummary {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_definition_has_no_grants() {
        let def = AgentDefinition::default();
        let summary = def.trust_summary();
        assert!(!summary.elevated);
        assert!(summary.is_empty());
    }

    #[test]
    fn plan_mode_not_elevated() {
        let def = AgentDefinition {
            permission_mode: Some("plan".to_string()),
            ..Default::default()
        };
        let summary = def.trust_summary();
        assert!(!summary.elevated, "plan mode should not be elevated");
    }

    #[test]
    fn implement_mode_is_elevated() {
        let def = AgentDefinition {
            permission_mode: Some("implement".to_string()),
            ..Default::default()
        };
        let summary = def.trust_summary();
        assert!(summary.elevated);
        assert!(summary.items.iter().any(|i| i.contains("implement")));
    }

    #[test]
    fn mcp_servers_are_elevated() {
        let mut mcp = BTreeMap::new();
        mcp.insert(
            "socraticode".to_string(),
            McpServerConfig {
                command: "npx".to_string(),
                args: Some(vec!["-y".to_string(), "socraticode".to_string()]),
                env: None,
                enabled: None,
                agents: None,
                requires: None,
            },
        );
        let def = AgentDefinition {
            bootstrap: Some(AgentDefinitionBootstrap {
                mcp,
                ..Default::default()
            }),
            ..Default::default()
        };
        let summary = def.trust_summary();
        assert!(summary.elevated);
        assert!(summary.items.iter().any(|i| i.contains("socraticode")));
    }

    #[test]
    fn resolve_template_prefers_inline() {
        let def = AgentDefinition {
            prompt_template: Some("inline template".to_string()),
            prompt_ref: Some("some-ref".to_string()),
            ..Default::default()
        };
        let defs = BTreeMap::new();
        assert_eq!(def.resolve_template(&defs), Some("inline template"));
    }

    #[test]
    fn resolve_template_falls_back_to_ref() {
        let def = AgentDefinition {
            prompt_ref: Some("my-prompt".to_string()),
            ..Default::default()
        };
        let mut defs = BTreeMap::new();
        defs.insert(
            "my-prompt".to_string(),
            crate::prompt::PromptTemplate {
                description: None,
                content: "referenced template".to_string(),
                required_variables: vec![],
            },
        );
        assert_eq!(def.resolve_template(&defs), Some("referenced template"));
    }

    #[test]
    fn resolve_template_returns_none_when_neither_set() {
        let def = AgentDefinition::default();
        let defs = BTreeMap::new();
        assert_eq!(def.resolve_template(&defs), None);
    }
}
