//! The runtime workmux has always had: a git worktree, a multiplexer window,
//! an agent CLI, and hook-driven status.
//!
//! This is a wrapper, not a rewrite. Creation goes through `workmux add` and
//! input through `workmux send` — exactly the calls the orchestration loop
//! already makes — and status comes from the same reconciled [`StateStore`] the
//! dashboard and sidebar read. Behaviour is meant to be indistinguishable from
//! before the trait existed.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{Result, bail};

use super::{
    AgentRef, AgentRuntime, Feature, LOCAL, RuntimeAgent, RuntimeFeatures, RuntimeHealth,
    RuntimeStatus, StartRequest, unsupported,
};
use crate::multiplexer::{create_backend, detect_backend};
use crate::state::StateStore;

pub struct LocalRuntime;

impl LocalRuntime {
    pub fn new() -> Self {
        Self
    }

    /// Reconciled agents from the shared state store, which is already the
    /// source of truth for every local-agent consumer.
    fn reconciled() -> Result<Vec<crate::multiplexer::AgentPane>> {
        let mux = create_backend(detect_backend());
        StateStore::new()?.load_reconciled_agents(mux.as_ref())
    }
}

impl Default for LocalRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRuntime for LocalRuntime {
    fn name(&self) -> &str {
        LOCAL
    }

    fn features(&self) -> RuntimeFeatures {
        RuntimeFeatures {
            panes: true,
            owns_worktree: true,
            send: true,
            freeze: true,
            fork: true,
            // Status arrives via hooks writing state files, which workmux polls.
            events: false,
            // Importing a foreign session into a tmux pane would mean taking
            // over a process this runtime did not start. It cannot.
            import: false,
        }
    }

    fn health(&self) -> RuntimeHealth {
        let mux = create_backend(detect_backend());
        match mux.is_running() {
            Ok(true) => RuntimeHealth::Ok,
            Ok(false) => {
                RuntimeHealth::Unavailable(format!("{} server is not running", mux.name()))
            }
            Err(e) => RuntimeHealth::Unavailable(format!("{}: {e}", mux.name())),
        }
    }

    fn start(&self, req: &StartRequest) -> Result<AgentRef> {
        if req.handle.is_empty() {
            bail!("local runtime: agent handle is required");
        }
        let mut cmd = Command::new("workmux");
        cmd.current_dir(&req.project_root).arg("add").arg(&req.handle);
        if let Some(kind) = &req.kind {
            cmd.arg("--agent").arg(kind);
        }
        if let Some(prompt) = &req.prompt {
            cmd.arg("--prompt").arg(prompt);
        }
        let out = cmd.output()?;
        if !out.status.success() {
            bail!(
                "local runtime: `workmux add {}` failed: {}",
                req.handle,
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }

        // The pane is the agent's identity here. It appears in the state store
        // once its process is visible, so resolve by the worktree we just made.
        let want = req.handle.as_str();
        let pane = Self::reconciled()?
            .into_iter()
            .find(|a| {
                a.window_name.ends_with(want)
                    || a.path.file_name().is_some_and(|n| n == want)
            })
            .map(|a| a.pane_id);
        match pane {
            Some(id) => Ok(AgentRef::new(LOCAL, id)),
            // The worktree exists but the agent process has not surfaced yet;
            // the handle is a stable stand-in until reconciliation adopts it.
            None => Ok(AgentRef::new(LOCAL, want)),
        }
    }

    fn send(&self, agent: &AgentRef, text: &str) -> Result<()> {
        let mux = create_backend(detect_backend());
        mux.send_keys(&agent.id, text)
    }

    fn status(&self, agent: &AgentRef) -> Result<RuntimeStatus> {
        let found = Self::reconciled()?
            .into_iter()
            .find(|a| a.pane_id == agent.id);
        Ok(match found {
            // A pane with no status yet is adopted but pre-hook: starting, not
            // unknown — the distinction matters to callers that wait on it.
            Some(a) => a.status.map(RuntimeStatus::from).unwrap_or(RuntimeStatus::Starting),
            None => RuntimeStatus::Unknown,
        })
    }

    fn list(&self) -> Result<Vec<RuntimeAgent>> {
        Ok(Self::reconciled()?
            .into_iter()
            .map(|a| RuntimeAgent {
                reference: AgentRef::new(LOCAL, a.pane_id),
                status: a.status.map(RuntimeStatus::from).unwrap_or(RuntimeStatus::Starting),
                workdir: Some(PathBuf::from(a.path)),
                title: a.pane_title,
                kind: a.agent_kind,
            })
            .collect())
    }

    fn stop(&self, agent: &AgentRef) -> Result<()> {
        if !self.features().has(Feature::Panes) {
            return Err(unsupported(self.name(), Feature::Panes));
        }
        let mux = create_backend(detect_backend());
        // Killing an already-gone pane is not a failure: stop is idempotent.
        let _ = mux.kill_pane(&agent.id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_runtime_declares_the_pane_features() {
        let f = LocalRuntime::new().features();
        assert!(f.panes && f.owns_worktree && f.send && f.freeze && f.fork);
        assert!(!f.events, "local status is polled from state files, not pushed");
    }

    #[test]
    fn local_runtime_is_named_local() {
        assert_eq!(LocalRuntime::new().name(), LOCAL);
    }

    #[test]
    fn start_requires_a_handle() {
        let err = LocalRuntime::new()
            .start(&StartRequest::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("handle is required"), "{err}");
    }
}
