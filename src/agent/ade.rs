//! Agent development environments: managers that run their own daemon and own
//! the agent process, with their own desktop, web, and phone clients.
//!
//! Muxix drives them through their CLI. That keeps zero protocol code here
//! and survives the manager's schema churn, at the cost of polled status
//! instead of pushed events — the same trade muxix already makes with tmux
//! and git. A streaming implementation can replace this behind the same
//! [`AgentRuntime`] trait without touching callers.
//!
//! No manager is hardcoded. An ADE is described by config
//! ([`crate::config::AdeConfig`]): the command to invoke and the subcommands to
//! use. Paseo ships as a default; another manager is a config block, and a
//! manager that does not fit the shape is a new trait impl.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Result, bail};
use serde_json::Value;

use crate::agent::runtime::{
    AgentRef, AgentRuntime, RuntimeAgent, RuntimeFeatures, RuntimeHealth, RuntimeStatus,
    StartRequest,
};
use crate::config::AdeConfig;

pub struct AdeRuntime {
    name: String,
    cfg: AdeConfig,
}

impl AdeRuntime {
    pub fn new(name: String, cfg: AdeConfig) -> Self {
        Self { name, cfg }
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let out = Command::new(&self.cfg.command)
            .args(args)
            .output()
            .map_err(|e| anyhow::anyhow!("{}: {e}", self.cfg.command))?;
        if !out.status.success() {
            bail!(
                "{} {}: {}",
                self.cfg.command,
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Parse the manager's JSON list output into agents.
    ///
    /// Accepts either a bare array or an object wrapping one, because managers
    /// differ and both shapes are common.
    fn parse_agents(&self, stdout: &str) -> Result<Vec<RuntimeAgent>> {
        let v: Value = serde_json::from_str(stdout.trim())
            .map_err(|e| anyhow::anyhow!("unparseable output from {}: {e}", self.cfg.command))?;
        let items = match &v {
            Value::Array(a) => a.clone(),
            Value::Object(o) => o
                .values()
                .find_map(|x| x.as_array().cloned())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        Ok(items
            .iter()
            .filter_map(|item| {
                let id = item
                    .get(&self.cfg.id_field)
                    .and_then(|x| x.as_str())
                    .or_else(|| item.get("id").and_then(|x| x.as_str()))?;
                Some(RuntimeAgent {
                    reference: AgentRef::new(&self.name, id),
                    status: self.map_status(
                        item.get(&self.cfg.status_field)
                            .and_then(|x| x.as_str())
                            .unwrap_or(""),
                    ),
                    workdir: item
                        .get("path")
                        .or_else(|| item.get("cwd"))
                        .and_then(|x| x.as_str())
                        .map(PathBuf::from),
                    title: item
                        .get("title")
                        .or_else(|| item.get("name"))
                        .and_then(|x| x.as_str())
                        .map(str::to_string),
                    kind: item
                        .get("provider")
                        .or_else(|| item.get("agent"))
                        .and_then(|x| x.as_str())
                        .map(str::to_string),
                })
            })
            .collect())
    }

    /// Map a manager's state name onto the neutral set. An unrecognised state
    /// becomes `Unknown` rather than a guess.
    fn map_status(&self, raw: &str) -> RuntimeStatus {
        let raw = raw.trim().to_ascii_lowercase();
        if raw.is_empty() {
            return RuntimeStatus::Unknown;
        }
        for (neutral, names) in [
            (RuntimeStatus::Starting, &self.cfg.status_map.starting),
            (RuntimeStatus::Working, &self.cfg.status_map.working),
            (RuntimeStatus::Waiting, &self.cfg.status_map.waiting),
            (RuntimeStatus::Done, &self.cfg.status_map.done),
            (RuntimeStatus::Failed, &self.cfg.status_map.failed),
        ] {
            if names.iter().any(|n| n.eq_ignore_ascii_case(&raw)) {
                return neutral;
            }
        }
        RuntimeStatus::Unknown
    }

    fn args_for(&self, template: &[String], subs: &[(&str, &str)]) -> Vec<String> {
        template
            .iter()
            .map(|a| {
                let mut a = a.clone();
                for (k, v) in subs {
                    a = a.replace(&format!("{{{k}}}"), v);
                }
                a
            })
            .collect()
    }
}

impl AgentRuntime for AdeRuntime {
    fn name(&self) -> &str {
        &self.name
    }

    fn features(&self) -> RuntimeFeatures {
        RuntimeFeatures {
            // The manager owns the process; there is no pane muxix can focus,
            // capture, or freeze.
            panes: false,
            owns_worktree: self.cfg.owns_worktree,
            send: true,
            freeze: false,
            fork: false,
            events: false,
            // Only when the manager tells us how to import: an ADE that cannot
            // adopt a foreign session must not claim it can.
            import: !self.cfg.import_args.is_empty(),
        }
    }

    fn health(&self) -> RuntimeHealth {
        // A missing CLI is the common case and must read as "unavailable",
        // never as an agent failure.
        match Command::new(&self.cfg.command).arg("--version").output() {
            Ok(out) if out.status.success() => RuntimeHealth::Ok,
            Ok(out) => RuntimeHealth::Unavailable(format!(
                "`{} --version` failed: {}",
                self.cfg.command,
                String::from_utf8_lossy(&out.stderr).trim()
            )),
            Err(e) => {
                RuntimeHealth::Unavailable(format!("`{}` not runnable: {e}", self.cfg.command))
            }
        }
    }

    fn start(&self, req: &StartRequest) -> Result<AgentRef> {
        let prompt = req.prompt.clone().unwrap_or_default();
        let cwd = req
            .worktree
            .clone()
            .unwrap_or_else(|| req.project_root.clone());
        let args = self.args_for(
            &self.cfg.start_args,
            &[
                ("prompt", prompt.as_str()),
                ("handle", req.handle.as_str()),
                ("cwd", &cwd.to_string_lossy()),
                ("agent", req.kind.as_deref().unwrap_or("")),
            ],
        );
        let stdout = self.run(&args.iter().map(String::as_str).collect::<Vec<_>>())?;

        // The id may come back as bare text or inside JSON; accept both rather
        // than requiring every manager to agree.
        let id = match serde_json::from_str::<Value>(stdout.trim()) {
            Ok(v) => v
                .get(&self.cfg.id_field)
                .or_else(|| v.get("id"))
                .and_then(|x| x.as_str())
                .map(str::to_string),
            Err(_) => Some(stdout.trim().to_string()).filter(|s| !s.is_empty()),
        };
        match id {
            Some(id) => Ok(AgentRef::new(&self.name, id)),
            None => bail!(
                "{} did not report an agent id when starting '{}'",
                self.cfg.command,
                req.handle
            ),
        }
    }

    fn send(&self, agent: &AgentRef, text: &str) -> Result<()> {
        let args = self.args_for(
            &self.cfg.send_args,
            &[("id", agent.id.as_str()), ("text", text)],
        );
        self.run(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        Ok(())
    }

    fn status(&self, agent: &AgentRef) -> Result<RuntimeStatus> {
        Ok(self
            .list()?
            .into_iter()
            .find(|a| a.reference == *agent)
            .map(|a| a.status)
            // Gone from the manager's own listing is not a crash — it is gone.
            .unwrap_or(RuntimeStatus::Unknown))
    }

    fn list(&self) -> Result<Vec<RuntimeAgent>> {
        let args = self.args_for(&self.cfg.list_args, &[]);
        let stdout = self.run(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        self.parse_agents(&stdout)
    }

    fn import(&self, session: &crate::agent::runtime::ExternalSession) -> Result<AgentRef> {
        if self.cfg.import_args.is_empty() {
            return Err(crate::agent::runtime::unsupported(
                &self.name,
                crate::agent::runtime::Feature::Import,
            ));
        }
        let args = self.args_for(
            &self.cfg.import_args,
            &[
                ("session", session.session_id.as_str()),
                ("provider", session.provider.as_str()),
                ("cwd", &session.cwd.to_string_lossy()),
            ],
        );
        let stdout = self.run(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        let id = match serde_json::from_str::<Value>(stdout.trim()) {
            Ok(v) => v
                .get(&self.cfg.id_field)
                .or_else(|| v.get("agentId"))
                .or_else(|| v.get("id"))
                .and_then(|x| x.as_str())
                .map(str::to_string),
            Err(_) => Some(stdout.trim().to_string()).filter(|s| !s.is_empty()),
        };
        match id {
            Some(id) => Ok(AgentRef::new(&self.name, id)),
            None => bail!(
                "{} did not report an agent id when importing session {}",
                self.cfg.command,
                session.session_id
            ),
        }
    }

    fn stop(&self, agent: &AgentRef) -> Result<()> {
        let args = self.args_for(&self.cfg.stop_args, &[("id", agent.id.as_str())]);
        // Stopping an agent the manager already dropped is not an error.
        let _ = self.run(&args.iter().map(String::as_str).collect::<Vec<_>>());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdeConfig;

    fn runtime() -> AdeRuntime {
        AdeRuntime::new("paseo".to_string(), AdeConfig::default())
    }

    #[test]
    fn agents_parse_from_a_bare_array() {
        let r = runtime();
        let out = r#"[{"id":"a1","status":"running","path":"/p","title":"t"}]"#;
        let agents = r.parse_agents(out).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].reference, AgentRef::new("paseo", "a1"));
        assert_eq!(agents[0].status, RuntimeStatus::Working);
        assert_eq!(agents[0].workdir, Some(PathBuf::from("/p")));
    }

    #[test]
    fn agents_parse_from_a_wrapped_array() {
        let r = runtime();
        let agents = r.parse_agents(r#"{"agents":[{"id":"a1"}]}"#).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].reference.id, "a1");
    }

    #[test]
    fn unparseable_output_is_an_error_not_an_empty_list() {
        // Silently returning no agents would look like "the manager has none",
        // which would strand every agent it actually owns.
        let err = runtime().parse_agents("not json").unwrap_err().to_string();
        assert!(err.contains("unparseable output"), "{err}");
    }

    #[test]
    fn paseo_lifecycle_states_all_map() {
        // The real set from packages/protocol/src/agent-lifecycle.ts. If Paseo
        // adds a state, this is where it should fail rather than silently
        // becoming `unknown` in the UI.
        let r = runtime();
        assert_eq!(r.map_status("initializing"), RuntimeStatus::Starting);
        assert_eq!(r.map_status("running"), RuntimeStatus::Working);
        assert_eq!(r.map_status("idle"), RuntimeStatus::Waiting);
        assert_eq!(r.map_status("closed"), RuntimeStatus::Done);
        assert_eq!(r.map_status("error"), RuntimeStatus::Failed);
    }

    #[test]
    fn a_failure_never_reads_as_completion() {
        // Mapping `error` onto `done` would render broken work as finished.
        let r = runtime();
        assert_ne!(r.map_status("error"), RuntimeStatus::Done);
    }

    #[test]
    fn unknown_states_are_not_guessed() {
        let r = runtime();
        assert_eq!(r.map_status("banana"), RuntimeStatus::Unknown);
        assert_eq!(r.map_status(""), RuntimeStatus::Unknown);
    }

    #[test]
    fn ade_agents_have_no_pane_features() {
        let f = runtime().features();
        assert!(!f.panes, "an ADE agent has no pane muxix can drive");
        assert!(!f.freeze);
        assert!(f.send);
    }

    #[test]
    fn missing_cli_reads_as_unavailable() {
        let cfg = AdeConfig {
            command: "muxix-no-such-binary".to_string(),
            ..Default::default()
        };
        let r = AdeRuntime::new("ghost".to_string(), cfg);
        match r.health() {
            RuntimeHealth::Unavailable(reason) => {
                assert!(reason.contains("not runnable"), "{reason}")
            }
            RuntimeHealth::Ok => panic!("a missing CLI must not report healthy"),
        }
    }

    #[test]
    fn argument_templates_substitute_placeholders() {
        let r = runtime();
        let args = r.args_for(
            &[
                "agent".into(),
                "send".into(),
                "{id}".into(),
                "{text}".into(),
            ],
            &[("id", "a1"), ("text", "hello")],
        );
        assert_eq!(args, vec!["agent", "send", "a1", "hello"]);
    }
}
