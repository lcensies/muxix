use super::types::{OrgPolicy, PolicyViolation, ViolationSeverity};

/// Check a resolved config against an org policy, returning any violations.
///
/// This validates only; it no longer mutates the config. A policy's `defaults`
/// and `locked` values are applied as ordinary config layers during resolution
/// (see `provision::layers`), which is what lets `workmux config resolve
/// --explain` attribute a value to the policy instead of it appearing from
/// nowhere after the merge.
///
/// What remains here are the deny-lists — forbidden MCP commands, allowlists
/// for MCP servers, providers and agent kinds. Those are assertions about the
/// final config, not values to merge into it, so they have no layer to live in.
pub fn validate_policy(config: &crate::config::Config, policy: &OrgPolicy) -> Vec<PolicyViolation> {
    let mut violations = Vec::new();
    let severity = if policy.violation_severity == "error" {
        ViolationSeverity::Error
    } else {
        ViolationSeverity::Warn
    };

    // --- forbidden MCP commands ---
    if !policy.forbidden_mcp_commands.is_empty() {
        if let Some(mcp) = &config.mcp {
            for (name, server) in mcp {
                for forbidden in &policy.forbidden_mcp_commands {
                    if server.command.contains(forbidden.as_str()) {
                        violations.push(PolicyViolation {
                            field: format!("mcp.{}.command", name),
                            message: format!(
                                "MCP server command contains forbidden pattern: {}",
                                forbidden
                            ),
                            severity: severity.clone(),
                        });
                    }
                }
            }
        }
    }

    // --- MCP server allowlist ---
    if !policy.allowed_mcp_servers.is_empty() {
        if let Some(mcp) = &config.mcp {
            for name in mcp.keys() {
                if !policy.allowed_mcp_servers.iter().any(|a| a == name) {
                    violations.push(PolicyViolation {
                        field: format!("mcp.{}", name),
                        message: format!("MCP server '{}' is not in the org allowlist", name),
                        severity: severity.clone(),
                    });
                }
            }
        }
    }

    // --- Provider allowlist ---
    if policy.deny_external_providers && !policy.allowed_providers.is_empty() {
        if let Some(providers) = &config.providers {
            let allowed: Vec<&str> = policy.allowed_providers.iter().map(|p| p.name.as_str()).collect();
            for name in providers.keys() {
                if !allowed.contains(&name.as_str()) {
                    violations.push(PolicyViolation {
                        field: format!("providers.{}", name),
                        message: format!(
                            "provider '{}' not in org allowlist (deny_external_providers=true)",
                            name
                        ),
                        severity: severity.clone(),
                    });
                }
            }
        }
    }

    // --- allowed agent kinds ---
    if !policy.allowed_agent_kinds.is_empty() {
        if let Some(agent) = &config.agent {
            let stem = agent
                .split_whitespace()
                .next()
                .and_then(|s| s.rsplit('/').next())
                .unwrap_or(agent.as_str());
            if !policy
                .allowed_agent_kinds
                .iter()
                .any(|k| k.eq_ignore_ascii_case(stem))
            {
                violations.push(PolicyViolation {
                    field: "agent".into(),
                    message: format!(
                        "agent '{}' not in org allowlist: {:?}",
                        stem, policy.allowed_agent_kinds
                    ),
                    severity: severity.clone(),
                });
            }
        }
    }

    violations
}

/// Emit violations to tracing + stderr (when running in a TTY).
pub fn report_violations(violations: &[PolicyViolation]) {
    for v in violations {
        let msg = format!("[org policy] {}: {}", v.field, v.message);
        tracing::warn!("{}", msg);
        if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
            eprintln!("warning: {}", msg);
        }
    }
}
