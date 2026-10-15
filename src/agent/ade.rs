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

#[derive(Clone)]
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
                        // `agent ls` reports provider and model as one field
                        // (`pi/anthropic/claude-opus-5-5`) while `inspect` splits
                        // them. Keep the provider, which is what selects a profile.
                        .map(|s| s.split('/').next().unwrap_or(s).to_string()),
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

    /// The ADE's executable, for callers that spawn it themselves (a streaming
    /// view cannot go through [`AdeRuntime::run`], which waits for exit).
    pub fn command(&self) -> &str {
        &self.cfg.command
    }

    /// Run one rendered client command, returning its stdout.
    pub fn run_client(&self, args: &[String]) -> Result<String> {
        self.run(&args.iter().map(String::as_str).collect::<Vec<_>>())
    }

    /// This ADE's view configuration, absent when it declares none.
    pub fn view(&self) -> Option<&crate::config::AdeViewConfig> {
        self.cfg.view.as_ref()
    }

    /// Render one of the view's command templates. Callers pass the template
    /// from [`AdeRuntime::view`] so one renderer serves every client command
    /// rather than a method per subcommand.
    pub fn render_args(&self, template: &[String], subs: &[(&str, &str)]) -> Vec<String> {
        self.args_for(template, subs)
    }

    /// The command that streams one session's output, for a view's body.
    pub fn view_command(&self, agent: &AgentRef) -> Option<Vec<String>> {
        let v = self.cfg.view.as_ref()?;
        if v.log_args.is_empty() {
            return None;
        }
        Some(self.args_for(&v.log_args, &[("id", agent.id.as_str())]))
    }

    /// One session's current state, for the chrome around a view.
    pub fn view_state(&self, agent: &AgentRef) -> Result<AdeAgentState> {
        let Some(v) = self.cfg.view.as_ref().filter(|v| !v.state_args.is_empty()) else {
            bail!("runtime '{}' declares no session state command", self.name);
        };
        let args = self.args_for(&v.state_args, &[("id", agent.id.as_str())]);
        let out = self.run(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
        let json: Value = serde_json::from_str(out.trim())
            .map_err(|e| anyhow::anyhow!("{}: unparseable state output: {e}", self.cfg.command))?;

        let status = json_get_ci(&json, &self.cfg.status_field)
            .and_then(Value::as_str)
            .map(|s| self.map_status(s))
            .unwrap_or(RuntimeStatus::Unknown);

        Ok(AdeAgentState {
            status,
            mode: json_get_ci(&json, "mode")
                .and_then(Value::as_str)
                .map(str::to_string),
            model: json_get_ci(&json, "model")
                .and_then(Value::as_str)
                .map(str::to_string),
            // Absent is zero pending, but a present non-array is not: that is a
            // shape this code does not understand, and reporting "no pending
            // permissions" would hide a prompt the session is blocked on.
            pending_permissions: match json_get_ci(&json, "pendingPermissions") {
                None | Some(Value::Null) => Some(0),
                Some(Value::Array(a)) => Some(a.len()),
                Some(_) => None,
            },
            cwd: json_get_ci(&json, "cwd")
                .and_then(Value::as_str)
                .map(PathBuf::from),
            provider: json_get_ci(&json, "provider")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    /// The provider session handle for one agent — the id its own CLI resumes.
    ///
    /// `handle_args` wins when the ADE reports the handle itself; otherwise the
    /// handle is read out of the ADE's own state file. A missing file, bad JSON,
    /// or absent field is an error naming both path and field: an empty handle
    /// passed onward would be worse than no handle, because an ADE that does not
    /// validate what it is given turns it into a junk agent.
    pub fn native_handle(&self, agent: &AgentRef, cwd: &std::path::Path) -> Result<String> {
        let Some(v) = self.cfg.view.as_ref() else {
            bail!("runtime '{}' declares no view block", self.name);
        };
        if v.handle_field.is_empty() {
            bail!("runtime '{}' declares no handle_field", self.name);
        }

        let (json, source) = if !v.handle_args.is_empty() {
            let args = self.args_for(&v.handle_args, &[("id", agent.id.as_str())]);
            let out = self.run(&args.iter().map(String::as_str).collect::<Vec<_>>())?;
            let source = format!("{} {}", self.cfg.command, args.join(" "));
            let json = serde_json::from_str::<Value>(out.trim())
                .map_err(|e| anyhow::anyhow!("{source}: unparseable output: {e}"))?;
            (json, source)
        } else if !v.handle_file.is_empty() {
            let rendered = self
                .args_for(
                    std::slice::from_ref(&v.handle_file),
                    &[
                        ("id", agent.id.as_str()),
                        ("cwd_slug", cwd_slug(cwd).as_str()),
                    ],
                )
                .join("");
            let path = crate::util::expand_tilde(&rendered);
            let source = path.display().to_string();
            let raw =
                std::fs::read_to_string(&path).map_err(|e| anyhow::anyhow!("{source}: {e}"))?;
            let json = serde_json::from_str::<Value>(raw.trim())
                .map_err(|e| anyhow::anyhow!("{source}: unparseable state file: {e}"))?;
            (json, source)
        } else {
            bail!(
                "runtime '{}' declares no handle source (handle_args or handle_file)",
                self.name
            );
        };

        json_dotted(&json, &v.handle_field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .map(str::to_string)
            .ok_or_else(|| anyhow::anyhow!("{source}: no '{}' in that output", v.handle_field))
    }
}

/// One ADE session's state, for the chrome muxix renders around a view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdeAgentState {
    pub status: RuntimeStatus,
    pub mode: Option<String>,
    pub model: Option<String>,
    /// Pending permission requests, or `None` when the ADE reported that field
    /// in a shape this code cannot count. `None` is not zero: a session blocked
    /// on a prompt must not render as having nothing to answer.
    pub pending_permissions: Option<usize>,
    /// The session's working directory, as the ADE reports it. Needed both to
    /// render a per-directory state path and to resume the session in the
    /// directory it belongs to.
    pub cwd: Option<PathBuf>,
    /// The ADE's provider id (`claude`, `codex`, …), which selects the agent
    /// profile whose resume flag takeover uses.
    pub provider: Option<String>,
}

/// Look up a JSON key, exactly first and then ignoring case.
///
/// One ADE answers with different casing per subcommand — paseo's `agent ls`
/// emits `status` while `inspect` emits `Status` — so a single configured field
/// name has to match either. Exact wins, so a document carrying both keys is
/// read the way its author meant.
fn json_get_ci<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    let obj = v.as_object()?;
    if let Some(found) = obj.get(key) {
        return Some(found);
    }
    obj.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}

/// Walk a dotted path such as `persistence.nativeHandle`, case-insensitively at
/// each step.
fn json_dotted<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.')
        .filter(|s| !s.is_empty())
        .try_fold(v, |cur, seg| json_get_ci(cur, seg))
}

/// Slug a directory the way an ADE keys its own per-directory state.
///
/// Paseo drops the leading separator and turns the rest into dashes:
/// `/home/u/repos/muxix` becomes `home-u-repos-muxix`. Deliberately not the
/// `slug` crate — that lowercases and strips dots, and these names keep both
/// (`home-u-.paseo-worktrees-x` is a real directory).
fn cwd_slug(dir: &std::path::Path) -> String {
    dir.to_string_lossy()
        .trim_start_matches('/')
        .replace('/', "-")
}

/// Build the runtime for one configured ADE by name.
///
/// The registry hands out `Arc<dyn AgentRuntime>`, which cannot reach the view
/// and handle methods — they are inherent to this type rather than widening the
/// trait for one consumer.
pub fn for_name(cfg: &crate::config::Config, name: &str) -> Option<AdeRuntime> {
    crate::projects::sync::effective_ades(cfg)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(n, c)| AdeRuntime::new(n, c))
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
    fn a_providers_model_is_not_mistaken_for_its_name() {
        // `agent ls` reports one joined field (`pi/anthropic/claude-opus-5-5`)
        // while `inspect` reports `Provider` and `Model` separately. The joined
        // form selects no agent profile, so takeover would refuse a session it
        // can actually resume.
        let r = runtime();
        let out = r#"[{"id":"a1","provider":"pi/anthropic/claude-opus-5-5"},
                      {"id":"a2","provider":"claude"}]"#;
        let agents = r.parse_agents(out).unwrap();
        assert_eq!(agents[0].kind.as_deref(), Some("pi"));
        assert_eq!(agents[1].kind.as_deref(), Some("claude"));
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

    fn handle_fixture() -> &'static str {
        r#"{
          "id": "cc842c69-3470-4ad5-bb93-a13e4dfbca86",
          "provider": "claude",
          "persistence": {
            "provider": "claude",
            "sessionId": "89430a6d-0049-4809-bf4d-a263bc6c4661",
            "nativeHandle": "89430a6d-0049-4809-bf4d-a263bc6c4661"
          }
        }"#
    }

    /// Write a fixture at the exact path the preset template renders, so the
    /// `{cwd_slug}` rendering is exercised rather than bypassed.
    fn runtime_with_handle_file(dir: &std::path::Path, cwd: &str, body: &str) -> AdeRuntime {
        let slug = cwd.trim_start_matches('/').replace('/', "-");
        let agents = dir.join("agents").join(&slug);
        std::fs::create_dir_all(&agents).unwrap();
        std::fs::write(agents.join("agt_1.json"), body).unwrap();
        let mut cfg = AdeConfig::paseo();
        let view = cfg.view.as_mut().unwrap();
        view.handle_file = format!("{}/agents/{{cwd_slug}}/{{id}}.json", dir.display());
        AdeRuntime::new("paseo".to_string(), cfg)
    }

    #[test]
    fn native_handle_reads_the_dotted_field_from_the_state_file() {
        let tmp = tempfile::tempdir().unwrap();
        let r = runtime_with_handle_file(tmp.path(), "/home/u/repos/muxix", handle_fixture());
        let handle = r
            .native_handle(
                &AgentRef::new("paseo", "agt_1"),
                std::path::Path::new("/home/u/repos/muxix"),
            )
            .unwrap();
        assert_eq!(handle, "89430a6d-0049-4809-bf4d-a263bc6c4661");
    }

    #[test]
    fn a_missing_field_names_the_field_rather_than_returning_empty() {
        // An ADE that does not validate an imported handle turns a blank one
        // into a junk agent, so this must fail rather than yield "".
        let tmp = tempfile::tempdir().unwrap();
        let r = runtime_with_handle_file(tmp.path(), "/home/u/x", r#"{"persistence":{}}"#);
        let err = r
            .native_handle(
                &AgentRef::new("paseo", "agt_1"),
                std::path::Path::new("/home/u/x"),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("persistence.nativeHandle"), "{err}");
    }

    #[test]
    fn an_empty_handle_is_not_a_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let r = runtime_with_handle_file(
            tmp.path(),
            "/home/u/x",
            r#"{"persistence":{"nativeHandle":"   "}}"#,
        );
        assert!(
            r.native_handle(
                &AgentRef::new("paseo", "agt_1"),
                std::path::Path::new("/home/u/x")
            )
            .is_err()
        );
    }

    #[test]
    fn a_missing_state_file_names_the_path() {
        let tmp = tempfile::tempdir().unwrap();
        let r = runtime_with_handle_file(tmp.path(), "/home/u/x", handle_fixture());
        // Same runtime, different cwd: the rendered path points at no fixture.
        let err = r
            .native_handle(
                &AgentRef::new("paseo", "agt_1"),
                std::path::Path::new("/home/u/elsewhere"),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("home-u-elsewhere"), "{err}");
    }

    #[test]
    fn malformed_state_file_is_an_error_not_a_missing_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let r = runtime_with_handle_file(tmp.path(), "/home/u/x", "not json");
        let err = r
            .native_handle(
                &AgentRef::new("paseo", "agt_1"),
                std::path::Path::new("/home/u/x"),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains("unparseable state file"), "{err}");
    }

    #[test]
    fn cwd_slug_keeps_dots_and_drops_the_leading_separator() {
        // These are real paseo state directory names; the `slug` crate would
        // lowercase and strip the dot and break the lookup.
        assert_eq!(
            cwd_slug(std::path::Path::new(
                "/home/u/.paseo/worktrees/x-dry-dolphin"
            )),
            "home-u-.paseo-worktrees-x-dry-dolphin"
        );
        assert_eq!(cwd_slug(std::path::Path::new("/tmp")), "tmp");
    }

    #[test]
    fn a_runtime_with_no_handle_source_refuses_rather_than_guessing() {
        let mut cfg = AdeConfig::paseo();
        let view = cfg.view.as_mut().unwrap();
        view.handle_file.clear();
        view.handle_args.clear();
        let r = AdeRuntime::new("paseo".to_string(), cfg);
        let err = r
            .native_handle(&AgentRef::new("paseo", "a"), std::path::Path::new("/tmp"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("no handle source"), "{err}");
    }

    #[test]
    fn json_lookup_matches_either_casing_but_prefers_exact() {
        // paseo's `agent ls` emits `status`, `inspect` emits `Status`.
        let v: Value = serde_json::from_str(r#"{"Status":"idle"}"#).unwrap();
        assert_eq!(json_get_ci(&v, "status").unwrap(), "idle");
        let both: Value = serde_json::from_str(r#"{"status":"a","Status":"b"}"#).unwrap();
        assert_eq!(json_get_ci(&both, "status").unwrap(), "a");
    }

    #[test]
    fn view_command_renders_the_log_template() {
        let r = runtime();
        let args = r.view_command(&AgentRef::new("paseo", "agt_1")).unwrap();
        assert_eq!(args, vec!["agent", "logs", "agt_1", "-f"], "{args:?}");
    }

    #[test]
    fn view_command_is_absent_without_a_view_block() {
        let mut cfg = AdeConfig::paseo();
        cfg.view = None;
        let r = AdeRuntime::new("paseo".to_string(), cfg);
        assert!(r.view_command(&AgentRef::new("paseo", "a")).is_none());
    }

    #[test]
    fn the_paseo_preset_views_without_claiming_stdin() {
        // `attach` takes over the invoking terminal's stdin; a muxix-driven pane
        // needs it. Swapping the preset to attach would silently break the pane.
        let v = AdeConfig::paseo().view.unwrap();
        assert!(!v.log_args.contains(&"attach".to_string()), "{v:?}");
        assert_eq!(v.on_select, crate::config::AdeViewOnSelect::Tui);
    }

    #[test]
    fn claude_resumes_a_named_session_not_merely_the_last_one() {
        // `--continue` resumes whichever session was most recent, so using it
        // for takeover would silently resume the wrong transcript.
        let p = crate::agent::profile::resolve_profile(Some("claude"));
        assert_eq!(p.resume_session_flag(), Some("--resume"));
        assert_ne!(p.resume_session_flag(), p.continue_flag());
    }

    #[test]
    fn pi_resumes_by_session_path() {
        // paseo's nativeHandle for a pi agent is the .jsonl path, and `--session`
        // is the flag that accepts one. Without this, taking over any pi session
        // is refused -- which is what happened on a real session.
        let p = crate::agent::profile::resolve_profile(Some("pi"));
        assert_eq!(p.resume_session_flag(), Some("--session"));
        assert_ne!(p.resume_session_flag(), p.continue_flag());
    }

    #[test]
    fn an_unverified_provider_has_no_resume_flag() {
        // paseo configures providers muxix has no verified resume flag for
        // (`omp`, `antigravity`, `muse`). Takeover must refuse rather than
        // borrow a neighbour's flag.
        for provider in ["omp", "codex", "opencode"] {
            let p = crate::agent::profile::resolve_profile(Some(provider));
            assert_eq!(
                p.resume_session_flag(),
                None,
                "{provider} gained a resume flag without being verified"
            );
        }
    }
}
