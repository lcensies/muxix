//! Which runtime owns a given agent, and what runtimes exist.
//!
//! Selection order is per-task override → project config → global default,
//! which is [`LOCAL`]. An unknown or unhealthy runtime is an error, never a
//! silent fall back to local: running work locally when the user asked for a
//! remote-reachable manager loses exactly the property they selected it for,
//! and they would not find out until they reached for their phone.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Result, bail};

use super::local::LocalRuntime;
use super::{AgentRuntime, LOCAL, RuntimeFeatures, RuntimeHealth};

/// One row of `workmux runtimes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeInfo {
    pub name: String,
    pub health: RuntimeHealth,
    pub features: RuntimeFeatures,
    /// True for the runtime that would be chosen with no configuration.
    pub is_default: bool,
}

/// Every runtime available to this workmux.
pub struct RuntimeRegistry {
    runtimes: BTreeMap<String, Arc<dyn AgentRuntime>>,
}

impl RuntimeRegistry {
    /// Registry holding only the built-in local runtime.
    pub fn new() -> Self {
        let mut runtimes: BTreeMap<String, Arc<dyn AgentRuntime>> = BTreeMap::new();
        runtimes.insert(LOCAL.to_string(), Arc::new(LocalRuntime::new()));
        Self { runtimes }
    }

    /// The registry as configured for a project: built-ins plus every ADE the
    /// config declares.
    pub fn for_config(cfg: &crate::config::Config) -> Self {
        let mut reg = Self::new();
        for (name, ade) in crate::projects::sync::effective_ades(cfg) {
            reg.register(Arc::new(super::super::ade::AdeRuntime::new(name, ade)));
        }
        reg
    }

    pub fn register(&mut self, runtime: Arc<dyn AgentRuntime>) {
        self.runtimes.insert(runtime.name().to_string(), runtime);
    }

    pub fn names(&self) -> Vec<String> {
        self.runtimes.keys().cloned().collect()
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn AgentRuntime>> {
        self.runtimes.get(name).cloned()
    }

    /// Resolve the runtime for one agent.
    ///
    /// `task_override` wins over `project_default`; absent both, [`LOCAL`].
    /// Fails loudly when the name is unknown or the runtime is unhealthy.
    pub fn select(
        &self,
        task_override: Option<&str>,
        project_default: Option<&str>,
    ) -> Result<Arc<dyn AgentRuntime>> {
        let name = task_override
            .or(project_default)
            .unwrap_or(LOCAL)
            .to_string();
        let Some(runtime) = self.get(&name) else {
            bail!(
                "unknown agent runtime '{name}' (registered: {})",
                self.names().join(", ")
            );
        };
        if let RuntimeHealth::Unavailable(reason) = runtime.health() {
            bail!("agent runtime '{name}' is unavailable: {reason}");
        }
        Ok(runtime)
    }

    /// The runtime that owns a given agent, by the runtime named in its ref.
    ///
    /// Operations route by the agent, not by configuration: an agent started in
    /// an ADE stays drivable from workmux even when this project's configured
    /// runtime is `local`, and vice versa. Selection decides who *creates* new
    /// agents; it does not decide who workmux is allowed to talk to.
    pub fn runtime_for(&self, agent: &crate::agent::runtime::AgentRef) -> Result<Arc<dyn AgentRuntime>> {
        self.get(&agent.runtime).ok_or_else(|| {
            anyhow::anyhow!(
                "agent {agent} belongs to unregistered runtime '{}'",
                agent.runtime
            )
        })
    }

    /// Agents from every healthy runtime.
    ///
    /// Process ownership is exclusive, but visibility is not: workmux shows
    /// what it started locally *and* what an ADE started, side by side. An
    /// unhealthy runtime contributes nothing and is reported, never silently
    /// treated as owning no agents.
    pub fn list_agents(&self) -> (Vec<crate::agent::runtime::RuntimeAgent>, Vec<(String, String)>) {
        let mut agents = Vec::new();
        let mut problems = Vec::new();
        for (name, runtime) in &self.runtimes {
            if let RuntimeHealth::Unavailable(reason) = runtime.health() {
                problems.push((name.clone(), reason));
                continue;
            }
            match runtime.list() {
                Ok(mut found) => agents.append(&mut found),
                Err(e) => problems.push((name.clone(), e.to_string())),
            }
        }
        agents.sort_by(|a, b| a.reference.cmp(&b.reference));
        (agents, problems)
    }

    /// Every runtime with its current health, for inspection.
    pub fn list(&self) -> Vec<RuntimeInfo> {
        self.runtimes
            .values()
            .map(|r| RuntimeInfo {
                name: r.name().to_string(),
                health: r.health(),
                features: r.features(),
                is_default: r.name() == LOCAL,
            })
            .collect()
    }
}

impl Default for RuntimeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// `workmux runtimes` — list runtimes, their health, and their features.
///
/// Succeeds even when a runtime is unhealthy: an unreachable manager is
/// information, not a failure of the listing.
pub fn print_runtimes(json: bool) -> Result<()> {
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    let list = RuntimeRegistry::for_config(&cfg).list();

    if json {
        let rows: Vec<serde_json::Value> = list
            .iter()
            .map(|r| {
                serde_json::json!({
                    "name": r.name,
                    "healthy": r.health.is_ok(),
                    "reason": r.health.reason(),
                    "features": r.features.names(),
                    "default": r.is_default,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }

    for r in list {
        let mark = if r.is_default { " (default)" } else { "" };
        match r.health.reason() {
            None => println!("{}{}  ok  [{}]", r.name, mark, r.features.names().join(", ")),
            Some(reason) => println!("{}{}  unavailable: {reason}", r.name, mark),
        }
    }
    Ok(())
}

/// Agents owned by runtimes other than `local`, as pane-shaped rows.
///
/// Shared by the sidebar and the dashboard so the two cannot drift on how a
/// pane-less agent is rendered. Unavailable runtimes contribute nothing; their
/// reasons are returned rather than swallowed.
pub fn foreign_agent_panes() -> (Vec<crate::multiplexer::AgentPane>, Vec<(String, String)>) {
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    let (agents, problems) = RuntimeRegistry::for_config(&cfg).list_agents();
    let panes = agents
        .iter()
        .filter(|a| !a.reference.is_local())
        .map(|a| a.to_agent_pane())
        .collect();
    (panes, problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::runtime::{AgentRef, RuntimeAgent, RuntimeStatus, StartRequest};

    struct Fake {
        name: String,
        health: RuntimeHealth,
        agents: Vec<String>,
    }

    impl AgentRuntime for Fake {
        fn name(&self) -> &str {
            &self.name
        }
        fn features(&self) -> RuntimeFeatures {
            RuntimeFeatures::NONE
        }
        fn health(&self) -> RuntimeHealth {
            self.health.clone()
        }
        fn start(&self, _: &StartRequest) -> Result<AgentRef> {
            Ok(AgentRef::new(&self.name, "1"))
        }
        fn status(&self, _: &AgentRef) -> Result<RuntimeStatus> {
            Ok(RuntimeStatus::Unknown)
        }
        fn list(&self) -> Result<Vec<RuntimeAgent>> {
            Ok(self
                .agents
                .iter()
                .map(|id| RuntimeAgent {
                    reference: AgentRef::new(&self.name, id),
                    status: RuntimeStatus::Working,
                    workdir: None,
                    title: None,
                    kind: None,
                })
                .collect())
        }
        fn stop(&self, _: &AgentRef) -> Result<()> {
            Ok(())
        }
    }

    fn registry_with(name: &str, health: RuntimeHealth) -> RuntimeRegistry {
        let mut reg = RuntimeRegistry::new();
        reg.register(Arc::new(Fake {
            name: name.to_string(),
            health,
            agents: Vec::new(),
        }));
        reg
    }

    #[test]
    fn local_is_registered_and_default() {
        let reg = RuntimeRegistry::new();
        assert!(reg.get(LOCAL).is_some());
        assert!(reg.list().iter().any(|r| r.is_default && r.name == LOCAL));
    }

    #[test]
    fn task_override_beats_project_default() {
        let reg = registry_with("remote", RuntimeHealth::Ok);
        let chosen = reg.select(Some("remote"), Some(LOCAL)).unwrap();
        assert_eq!(chosen.name(), "remote");
    }

    #[test]
    fn project_default_is_used_without_an_override() {
        let reg = registry_with("remote", RuntimeHealth::Ok);
        assert_eq!(reg.select(None, Some("remote")).unwrap().name(), "remote");
    }

    #[test]
    fn nothing_configured_selects_local() {
        let reg = registry_with("remote", RuntimeHealth::Ok);
        assert_eq!(reg.select(None, None).unwrap().name(), LOCAL);
    }

    #[test]
    fn unknown_runtime_names_the_registered_ones() {
        let reg = registry_with("remote", RuntimeHealth::Ok);
        let err = match reg.select(Some("nope"), None) {
            Err(e) => e.to_string(),
            Ok(r) => panic!("expected failure, got runtime {}", r.name()),
        };
        assert!(err.contains("unknown agent runtime 'nope'"), "{err}");
        assert!(err.contains("remote"), "{err}");
    }

    #[test]
    fn unhealthy_runtime_fails_instead_of_falling_back() {
        // The whole point of selecting a remote-capable runtime is remote
        // reach; quietly running locally would lose it without telling anyone.
        let reg = registry_with(
            "remote",
            RuntimeHealth::Unavailable("daemon unreachable".to_string()),
        );
        let err = match reg.select(Some("remote"), None) {
            Err(e) => e.to_string(),
            Ok(r) => panic!("expected failure, got runtime {}", r.name()),
        };
        assert!(err.contains("unavailable"), "{err}");
        assert!(err.contains("daemon unreachable"), "{err}");
    }

    #[test]
    fn agents_are_listed_across_every_runtime() {
        // Ownership of a process is exclusive; visibility is not. An agent
        // started in an ADE must show up even when this project creates its own
        // agents locally.
        let mut reg = RuntimeRegistry::new();
        reg.register(Arc::new(Fake {
            name: "remote".to_string(),
            health: RuntimeHealth::Ok,
            agents: vec!["a1".to_string(), "a2".to_string()],
        }));
        let (agents, problems) = reg.list_agents();
        let refs: Vec<String> = agents.iter().map(|a| a.reference.to_wire()).collect();
        assert!(refs.contains(&"remote:a1".to_string()), "{refs:?}");
        assert!(refs.contains(&"remote:a2".to_string()), "{refs:?}");
        assert!(problems.is_empty(), "{problems:?}");
    }

    #[test]
    fn an_unhealthy_runtime_is_reported_not_treated_as_empty() {
        let mut reg = RuntimeRegistry::new();
        reg.register(Arc::new(Fake {
            name: "remote".to_string(),
            health: RuntimeHealth::Unavailable("daemon down".to_string()),
            agents: vec!["a1".to_string()],
        }));
        let (_, problems) = reg.list_agents();
        assert!(problems.iter().any(|(n, r)| n == "remote" && r.contains("daemon down")));
    }

    #[test]
    fn operations_route_by_the_agents_own_runtime() {
        // The project may be configured for `local` and still drive an agent
        // that an ADE owns: selection decides who *creates*, not who we talk to.
        let reg = registry_with("remote", RuntimeHealth::Ok);
        let owned = AgentRef::new("remote", "a1");
        assert_eq!(reg.runtime_for(&owned).unwrap().name(), "remote");
        assert_eq!(
            reg.runtime_for(&AgentRef::new(LOCAL, "%1")).unwrap().name(),
            LOCAL
        );
    }

    #[test]
    fn an_agent_from_an_unregistered_runtime_is_an_explicit_error() {
        let reg = RuntimeRegistry::new();
        let err = match reg.runtime_for(&AgentRef::new("ghost", "a1")) {
            Err(e) => e.to_string(),
            Ok(r) => panic!("expected failure, got {}", r.name()),
        };
        assert!(err.contains("unregistered runtime 'ghost'"), "{err}");
    }

    #[test]
    fn listing_includes_unhealthy_runtimes() {
        let reg = registry_with(
            "remote",
            RuntimeHealth::Unavailable("not installed".to_string()),
        );
        let list = reg.list();
        let remote = list.iter().find(|r| r.name == "remote").unwrap();
        assert_eq!(remote.health.reason(), Some("not installed"));
    }
}