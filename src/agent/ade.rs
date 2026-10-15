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

    /// Resolve the ADE workspace for `dir`, creating one only when the ADE
    /// does not already track that directory.
    ///
    /// `paseo workspace create` is not idempotent — it hands back a second
    /// workspace for a path it already knows — so this lists first and matches
    /// by canonical path.
    fn ensure_workspace(&self, dir: &std::path::Path, name: &str) -> Result<Option<String>> {
        let Some(ws) = self.cfg.workspaces.as_ref().filter(|w| w.ensure) else {
            return Ok(None);
        };
        if ws.list_args.is_empty() || ws.add_args.is_empty() {
            return Ok(None);
        }
        let want = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());

        let listed = self.run(
            &self
                .args_for(&ws.list_args, &[])
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )?;
        let existing = serde_json::from_str::<Value>(listed.trim())
            .ok()
            .map(|v| match v {
                Value::Array(a) => a,
                Value::Object(o) => o
                    .values()
                    .find_map(|x| x.as_array().cloned())
                    .unwrap_or_default(),
                _ => Vec::new(),
            })
            .unwrap_or_default()
            .into_iter()
            .find(|w| {
                w.get(&ws.root_field)
                    .and_then(|r| r.as_str())
                    .map(|r| {
                        let p = PathBuf::from(r);
                        p.canonicalize().unwrap_or(p) == want
                    })
                    .unwrap_or(false)
            })
            .and_then(|w| {
                w.get(&ws.id_field)
                    .and_then(|i| i.as_str())
                    .map(str::to_string)
            });
        if let Some(id) = existing {
            return Ok(Some(id));
        }

        let created = self.run(
            &self
                .args_for(
                    &ws.add_args,
                    &[("root", &want.to_string_lossy()), ("name", name)],
                )
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        )?;
        let id = serde_json::from_str::<Value>(created.trim())
            .ok()
            .and_then(|v| {
                v.get(&ws.id_field)
                    .and_then(|i| i.as_str())
                    .map(str::to_string)
            })
            .or_else(|| Some(created.trim().to_string()).filter(|s| !s.is_empty()));
        Ok(id)
    }

    /// Resolve a workspace, or warn and carry on without one.
    ///
    /// The agent process is the valuable part and runs fine unregistered; what
    /// is lost is reach from the ADE's remote clients, which the user needs to
    /// know about rather than have the handoff refused over.
    fn workspace_or_warn(&self, dir: &std::path::Path, name: &str) -> Option<String> {
        match self.ensure_workspace(dir, name) {
            Ok(id) => id,
            Err(e) => {
                eprintln!(
                    "muxix: {} could not register a workspace for {}: {e}\n\
                     the agent will run but may not open in {}'s own clients",
                    self.name,
                    dir.display(),
                    self.name
                );
                None
            }
        }
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
        let rendered: Vec<String> = template
            .iter()
            .map(|a| {
                let mut a = a.clone();
                for (k, v) in subs {
                    a = a.replace(&format!("{{{k}}}"), v);
                }
                a
            })
            .collect();

        // `--workspace ""` is an argument error, so a flag whose value came out
        // empty drops as a pair. A bare positional that renders empty is left
        // alone: an empty prompt is still an argument the manager expects.
        let mut out: Vec<String> = Vec::with_capacity(rendered.len());
        for (i, arg) in rendered.iter().enumerate() {
            let emptied = arg.is_empty() && !template[i].is_empty();
            if emptied && out.last().is_some_and(|p| p.starts_with("--")) {
                out.pop();
                continue;
            }
            out.push(arg.clone());
        }
        out
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

        // Only a directory muxix owns needs registering. When the ADE created
        // the worktree itself it already knows the workspace, and a second one
        // over the same path is the duplicate this avoids.
        let workspace = match &req.worktree {
            Some(wt) => self.workspace_or_warn(wt, &req.handle).unwrap_or_default(),
            None => String::new(),
        };

        let args = self.args_for(
            &self.cfg.start_args,
            &[
                ("prompt", prompt.as_str()),
                ("handle", req.handle.as_str()),
                ("cwd", &cwd.to_string_lossy()),
                ("workspace", workspace.as_str()),
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

        // The adopting ADE resolves an imported session's workspace from its
        // path (`paseo agent import` has no workspace flag), so the workspace
        // has to exist before the import, not be named in it.
        let name = session
            .cwd
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| session.session_id.clone());
        let workspace = self
            .workspace_or_warn(&session.cwd, &name)
            .unwrap_or_default();

        let args = self.args_for(
            &self.cfg.import_args,
            &[
                ("session", session.session_id.as_str()),
                ("provider", session.provider.as_str()),
                ("cwd", &session.cwd.to_string_lossy()),
                ("workspace", workspace.as_str()),
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

    #[test]
    fn preset_import_calls_the_managers_import_command() {
        // A preset that imports by listing agents reports success without
        // adopting anything.
        let r = runtime();
        let args = r.args_for(
            &r.cfg.import_args.clone(),
            &[
                ("session", "sess_1"),
                ("provider", "claude"),
                ("cwd", "/tmp/wt"),
            ],
        );
        assert_eq!(
            args,
            vec![
                "agent",
                "import",
                "sess_1",
                "--provider",
                "claude",
                "--cwd",
                "/tmp/wt",
                "--json"
            ]
        );
    }

    /// An ADE stub: `echo` prints whatever template we hand it, which is the
    /// JSON the real manager would have printed.
    fn echoing(list: &str, add: &str) -> AdeRuntime {
        let mut cfg = AdeConfig::paseo();
        cfg.command = "echo".to_string();
        cfg.workspaces = Some(crate::config::AdeWorkspacesConfig {
            list_args: vec![list.to_string()],
            add_args: vec![add.to_string()],
            id_field: "workspaceId".to_string(),
            root_field: "cwd".to_string(),
            ensure: true,
        });
        AdeRuntime::new("paseo".to_string(), cfg)
    }

    #[test]
    fn a_directory_the_ade_already_tracks_is_reused() {
        let dir = std::env::temp_dir();
        let canonical = dir.canonicalize().unwrap();
        let r = echoing(
            &format!(
                r#"[{{"workspaceId":"wks_existing","cwd":"{}"}}]"#,
                canonical.display()
            ),
            r#"{"workspaceId":"wks_created"}"#,
        );
        assert_eq!(
            r.ensure_workspace(&dir, "fix-parser").unwrap().as_deref(),
            Some("wks_existing"),
            "a second workspace over a tracked path is the duplicate this avoids"
        );
    }

    #[test]
    fn an_unknown_directory_is_created_with_root_and_name() {
        let dir = std::env::temp_dir();
        let r = echoing("[]", r#"{"workspaceId":"{name} at {root}"}"#);
        let id = r.ensure_workspace(&dir, "fix-parser").unwrap().unwrap();
        let canonical = dir.canonicalize().unwrap();
        assert_eq!(id, format!("fix-parser at {}", canonical.display()));
    }

    #[test]
    fn an_ade_without_a_workspaces_block_resolves_nothing() {
        let mut cfg = AdeConfig::paseo();
        cfg.workspaces = None;
        let r = AdeRuntime::new("plain".to_string(), cfg);
        assert_eq!(
            r.ensure_workspace(&std::env::temp_dir(), "x").unwrap(),
            None
        );
    }

    #[test]
    fn ensure_false_resolves_nothing() {
        let r = echoing("[]", r#"{"workspaceId":"wks_created"}"#);
        let mut cfg = r.cfg.clone();
        cfg.workspaces.as_mut().unwrap().ensure = false;
        let r = AdeRuntime::new("paseo".to_string(), cfg);
        assert_eq!(
            r.ensure_workspace(&std::env::temp_dir(), "x").unwrap(),
            None
        );
    }

    #[test]
    fn an_unresolved_workspace_drops_its_flag_rather_than_passing_empty() {
        let r = runtime();
        let args = r.args_for(
            &r.cfg.start_args.clone(),
            &[("prompt", "fix it"), ("cwd", "/tmp/wt"), ("workspace", "")],
        );
        assert!(!args.contains(&"--workspace".to_string()), "{args:?}");
        assert!(!args.iter().any(String::is_empty), "{args:?}");

        let with = r.args_for(
            &r.cfg.start_args.clone(),
            &[
                ("prompt", "fix it"),
                ("cwd", "/tmp/wt"),
                ("workspace", "wks_1"),
            ],
        );
        assert_eq!(
            with,
            vec![
                "agent",
                "run",
                "--json",
                "--workspace",
                "wks_1",
                "--cwd",
                "/tmp/wt",
                "fix it"
            ]
        );
    }

    #[test]
    fn an_empty_prompt_is_still_passed() {
        // Dropping empties is for flag pairs only. A positional the manager
        // expects stays, blank or not.
        let r = runtime();
        let args = r.args_for(
            &["agent".into(), "run".into(), "{prompt}".into()],
            &[("prompt", "")],
        );
        assert_eq!(args, vec!["agent", "run", ""], "{args:?}");
    }
}
