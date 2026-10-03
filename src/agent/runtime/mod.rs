//! Where an agent runs, and who owns its process.
//!
//! Muxix already abstracts two *other* things about agents, and this is
//! neither of them:
//!
//! - [`crate::agent::profile::AgentProfile`] — one agent CLI's quirks (bang
//!   delay, prompt flag, skip-permissions flag) once it is running in a pane.
//! - [`crate::agent::definition::AgentDefinition`] — how a run is configured:
//!   model, permission mode, prompt template, bootstrap.
//!
//! Both assume muxix forked the agent into a multiplexer pane it owns. That
//! assumption is what [`AgentRuntime`] removes: an agent may instead be owned by
//! an external agent development environment (ADE) with its own daemon and its
//! own phone/web clients, in which case muxix drives it rather than spawning
//! it.
//!
//! Naming note: the declared abilities of a runtime are **features**, not
//! capabilities. "Capability" already means an orchestrator setup unit
//! (`pipeline::preflight`, `cap_store`) and the permissions an `AgentDefinition`
//! grants a model. A third meaning would be ambiguous.

pub mod local;
pub mod registry;

use std::fmt;
use std::path::PathBuf;

use anyhow::Result;

/// Name of the built-in runtime that owns agents in the local multiplexer.
pub const LOCAL: &str = "local";

/// A runtime-neutral handle for one agent.
///
/// `id` is opaque and owned by the runtime: a pane id for [`LOCAL`], the
/// manager's own agent id for an ADE. Consumers must not parse it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AgentRef {
    pub runtime: String,
    pub id: String,
}

impl AgentRef {
    pub fn new(runtime: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            runtime: runtime.into(),
            id: id.into(),
        }
    }

    /// Namespaced rendering (`<runtime>:<id>`) for wire fields that predate
    /// runtimes and carry a single string. Local agents render bare so existing
    /// consumers that expect a pane id keep working.
    pub fn to_wire(&self) -> String {
        if self.runtime == LOCAL {
            self.id.clone()
        } else {
            format!("{}:{}", self.runtime, self.id)
        }
    }

    /// Inverse of [`AgentRef::to_wire`].
    pub fn from_wire(s: &str) -> Self {
        match s.split_once(':') {
            // A tmux pane id is `%3` and a wezterm one is numeric, so a colon
            // means a namespaced ref, not a pane.
            Some((runtime, id)) if !runtime.is_empty() && !id.is_empty() => {
                AgentRef::new(runtime, id)
            }
            _ => AgentRef::new(LOCAL, s),
        }
    }

    pub fn is_local(&self) -> bool {
        self.runtime == LOCAL
    }
}

impl fmt::Display for AgentRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_wire())
    }
}

/// The one status set every runtime maps onto.
///
/// Mirrors [`crate::multiplexer::AgentStatus`] plus the two cases a remote
/// manager can report that a pane cannot: an agent that exists but has not
/// started, and a state with no neutral equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeStatus {
    /// Created but not yet running.
    Starting,
    /// Actively processing.
    Working,
    /// Needs user input.
    Waiting,
    /// Finished its turn.
    Done,
    /// Ended in failure. Distinct from `Done`: a manager that reports an error
    /// must not be rendered as successful completion.
    Failed,
    /// Reported something with no neutral equivalent. Never guessed at.
    Unknown,
}

impl RuntimeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeStatus::Starting => "starting",
            RuntimeStatus::Working => "working",
            RuntimeStatus::Waiting => "waiting",
            RuntimeStatus::Done => "done",
            RuntimeStatus::Failed => "failed",
            RuntimeStatus::Unknown => "unknown",
        }
    }
}

impl From<crate::multiplexer::AgentStatus> for RuntimeStatus {
    fn from(s: crate::multiplexer::AgentStatus) -> Self {
        match s {
            crate::multiplexer::AgentStatus::Working => RuntimeStatus::Working,
            crate::multiplexer::AgentStatus::Waiting => RuntimeStatus::Waiting,
            crate::multiplexer::AgentStatus::Done => RuntimeStatus::Done,
        }
    }
}

/// What a runtime can do. Callers check before offering an operation, and an
/// unsupported call fails naming the runtime and the feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeFeatures {
    /// Agents occupy a multiplexer pane that can be focused and captured.
    pub panes: bool,
    /// The runtime creates and owns the git worktree an agent runs in.
    pub owns_worktree: bool,
    /// Text can be sent to a running agent.
    pub send: bool,
    /// Agents can be frozen/thawed (SIGSTOP-style idle suspension).
    pub freeze: bool,
    /// A conversation can be forked into a new agent.
    pub fork: bool,
    /// Status arrives as pushed events rather than polling.
    pub events: bool,
    /// An agent owned by *another* runtime can be imported for visibility and
    /// control here, without moving the process.
    pub import: bool,
}

impl RuntimeFeatures {
    /// Nothing supported; implementations turn on what they have.
    #[allow(dead_code)]
    pub const NONE: RuntimeFeatures = RuntimeFeatures {
        panes: false,
        owns_worktree: false,
        send: false,
        freeze: false,
        fork: false,
        events: false,
        import: false,
    };

    pub fn has(&self, f: Feature) -> bool {
        match f {
            Feature::Panes => self.panes,
            Feature::OwnsWorktree => self.owns_worktree,
            Feature::Send => self.send,
            Feature::Freeze => self.freeze,
            Feature::Fork => self.fork,
            Feature::Events => self.events,
            Feature::Import => self.import,
        }
    }

    /// Names of the supported features, for `muxix runtimes`.
    pub fn names(&self) -> Vec<&'static str> {
        Feature::ALL
            .iter()
            .filter(|f| self.has(**f))
            .map(|f| f.as_str())
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    Panes,
    OwnsWorktree,
    Send,
    Freeze,
    Fork,
    Events,
    Import,
}

impl Feature {
    pub const ALL: [Feature; 7] = [
        Feature::Panes,
        Feature::OwnsWorktree,
        Feature::Send,
        Feature::Freeze,
        Feature::Fork,
        Feature::Events,
        Feature::Import,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Feature::Panes => "panes",
            Feature::OwnsWorktree => "owns-worktree",
            Feature::Send => "send",
            Feature::Freeze => "freeze",
            Feature::Fork => "fork",
            Feature::Events => "events",
            Feature::Import => "import",
        }
    }
}

/// Error for an operation the selected runtime does not support.
///
/// A silent no-op would be worse than an error: a `send` that quietly does
/// nothing because the agent lives in another manager looks like a delivered
/// message.
pub fn unsupported(runtime: &str, feature: Feature) -> anyhow::Error {
    anyhow::anyhow!(
        "runtime '{runtime}' does not support '{}'",
        feature.as_str()
    )
}

/// One agent as the runtime sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAgent {
    pub reference: AgentRef,
    pub status: RuntimeStatus,
    /// Directory the agent works in, when the runtime exposes one.
    pub workdir: Option<PathBuf>,
    /// Human-readable title, when the runtime exposes one.
    pub title: Option<String>,
    /// Agent kind (`claude`, `codex`, …) when known.
    pub kind: Option<String>,
}

impl RuntimeAgent {
    /// Render as an [`AgentPane`] so pane-shaped views (sidebar, dashboard) can
    /// show agents no pane backs.
    ///
    /// `pane_id` carries the namespaced reference and `runtime` is set, so every
    /// consumer can tell this apart from a real pane via
    /// [`AgentPane::has_pane`] before doing anything multiplexer-shaped.
    /// `pane_pid` and `window_id` stay empty, which is what the auto-freeze path
    /// already skips on.
    pub fn to_agent_pane(&self) -> crate::multiplexer::AgentPane {
        use crate::multiplexer::AgentStatus;
        crate::multiplexer::AgentPane {
            // The runtime stands in for the session and the agent's own title
            // for the window, so existing grouping still has keys to group by.
            session: self.reference.runtime.clone(),
            window_name: self
                .title
                .clone()
                .unwrap_or_else(|| self.reference.id.clone()),
            pane_id: self.reference.to_wire(),
            window_id: String::new(),
            path: self.workdir.clone().unwrap_or_default(),
            pane_title: self.title.clone(),
            status: match self.status {
                RuntimeStatus::Working => Some(AgentStatus::Working),
                RuntimeStatus::Waiting => Some(AgentStatus::Waiting),
                RuntimeStatus::Done => Some(AgentStatus::Done),
                // Starting, Failed, and Unknown have no pane-status equivalent.
                // Leaving it unset renders neutrally instead of misreporting —
                // a failed agent shown as Done would read as success.
                _ => None,
            },
            status_ts: None,
            updated_ts: None,
            window_cmd: None,
            agent_command: None,
            agent_kind: self.kind.clone(),
            pipeline_node_title: None,
            activity: None,
            pane_pid: 0,
            runtime: Some(self.reference.runtime.clone()),
        }
    }
}

/// What to start.
#[derive(Debug, Clone, Default)]
pub struct StartRequest {
    /// Project root the agent belongs to.
    pub project_root: PathBuf,
    /// Handle/name for the agent's worktree or session.
    pub handle: String,
    /// Initial prompt, if any.
    pub prompt: Option<String>,
    /// Agent kind to launch (`claude`, `codex`, …); runtime default if None.
    pub kind: Option<String>,
    /// Existing worktree to run in. When None the runtime creates one if it
    /// declares `owns_worktree`.
    pub worktree: Option<PathBuf>,
}

/// Health of a runtime: whether it can be used right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeHealth {
    Ok,
    /// Unusable, with the reason (missing CLI, daemon unreachable, bad output).
    Unavailable(String),
}

impl RuntimeHealth {
    pub fn is_ok(&self) -> bool {
        matches!(self, RuntimeHealth::Ok)
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            RuntimeHealth::Ok => None,
            RuntimeHealth::Unavailable(r) => Some(r),
        }
    }
}

/// Who owns an agent's process, and how muxix drives it.
pub trait AgentRuntime: Send + Sync {
    /// Registry name, e.g. `local` or `paseo`.
    fn name(&self) -> &str;

    /// What this runtime supports.
    fn features(&self) -> RuntimeFeatures;

    /// Whether the runtime is usable right now. Never panics or blocks long:
    /// this runs before every selection and in `muxix runtimes`.
    fn health(&self) -> RuntimeHealth;

    /// Create and start an agent.
    fn start(&self, req: &StartRequest) -> Result<AgentRef>;

    /// Send text to a running agent.
    fn send(&self, agent: &AgentRef, text: &str) -> Result<()> {
        let _ = (agent, text);
        Err(unsupported(self.name(), Feature::Send))
    }

    /// Current status of one agent.
    #[allow(dead_code)]
    fn status(&self, agent: &AgentRef) -> Result<RuntimeStatus>;

    /// Every agent this runtime currently owns. Authoritative: agents it does
    /// not list are gone, however muxix last recorded them.
    fn list(&self) -> Result<Vec<RuntimeAgent>>;

    /// Stop an agent. Stopping an already-gone agent is not an error.
    fn stop(&self, agent: &AgentRef) -> Result<()>;

    /// Take over *visibility and control* of a session another runtime's
    /// process owns, without moving the process.
    ///
    /// This is what makes ownership exclusive but management shared: a Claude
    /// session running in a local tmux pane can be imported into an ADE and
    /// driven from its phone client, while tmux still holds the process.
    #[allow(dead_code)]
    fn import(&self, session: &ExternalSession) -> Result<AgentRef> {
        let _ = session;
        Err(unsupported(self.name(), Feature::Import))
    }
}

/// A session owned by some other runtime, offered for import.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct ExternalSession {
    /// The provider's own session/thread id (e.g. a Claude Code session id).
    pub session_id: String,
    /// Provider name (`claude`, `codex`, …).
    pub provider: String,
    /// Working directory the session runs in.
    pub cwd: PathBuf,
    /// Human-readable title, when known.
    pub title: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_refs_render_bare_on_the_wire() {
        // Existing consumers read this field as a pane id; local agents must
        // keep looking exactly like they did.
        let r = AgentRef::new(LOCAL, "%3");
        assert_eq!(r.to_wire(), "%3");
        assert_eq!(AgentRef::from_wire("%3"), r);
    }

    #[test]
    fn external_refs_are_namespaced() {
        let r = AgentRef::new("paseo", "agt_123");
        assert_eq!(r.to_wire(), "paseo:agt_123");
        assert_eq!(AgentRef::from_wire("paseo:agt_123"), r);
        assert!(!r.is_local());
    }

    #[test]
    fn wire_roundtrip_is_stable_for_both() {
        for r in [AgentRef::new(LOCAL, "%12"), AgentRef::new("paseo", "x")] {
            assert_eq!(AgentRef::from_wire(&r.to_wire()), r);
        }
    }

    #[test]
    fn malformed_wire_values_fall_back_to_local() {
        assert_eq!(AgentRef::from_wire(":x"), AgentRef::new(LOCAL, ":x"));
        assert_eq!(AgentRef::from_wire("x:"), AgentRef::new(LOCAL, "x:"));
        assert_eq!(AgentRef::from_wire(""), AgentRef::new(LOCAL, ""));
    }

    #[test]
    fn multiplexer_status_maps_onto_the_neutral_set() {
        use crate::multiplexer::AgentStatus;
        assert_eq!(
            RuntimeStatus::from(AgentStatus::Working),
            RuntimeStatus::Working
        );
        assert_eq!(
            RuntimeStatus::from(AgentStatus::Waiting),
            RuntimeStatus::Waiting
        );
        assert_eq!(RuntimeStatus::from(AgentStatus::Done), RuntimeStatus::Done);
    }

    #[test]
    fn feature_names_list_only_what_is_supported() {
        let f = RuntimeFeatures {
            panes: true,
            send: true,
            ..RuntimeFeatures::NONE
        };
        assert_eq!(f.names(), vec!["panes", "send"]);
        assert!(f.has(Feature::Panes));
        assert!(!f.has(Feature::Freeze));
    }

    fn ade_agent(status: RuntimeStatus) -> RuntimeAgent {
        RuntimeAgent {
            reference: AgentRef::new("paseo", "agt_1"),
            status,
            workdir: Some(PathBuf::from("/p")),
            title: Some("fix the parser".to_string()),
            kind: Some("claude".to_string()),
        }
    }

    #[test]
    fn a_pane_less_agent_renders_without_claiming_a_pane() {
        let pane = ade_agent(RuntimeStatus::Working).to_agent_pane();
        assert!(!pane.has_pane());
        assert_eq!(pane.runtime_name(), "paseo");
        assert_eq!(pane.pane_id, "paseo:agt_1");
        // The auto-freeze path skips on exactly these two being empty.
        assert_eq!(pane.pane_pid, 0);
        assert!(pane.window_id.is_empty());
    }

    #[test]
    fn a_failed_agent_is_not_rendered_as_done() {
        // Forcing Failed into the pane status set would show broken work as
        // finished; unset renders neutrally instead.
        use crate::multiplexer::AgentStatus;
        assert_eq!(
            ade_agent(RuntimeStatus::Failed).to_agent_pane().status,
            None
        );
        assert_eq!(
            ade_agent(RuntimeStatus::Starting).to_agent_pane().status,
            None
        );
        assert_eq!(
            ade_agent(RuntimeStatus::Unknown).to_agent_pane().status,
            None
        );
        assert_eq!(
            ade_agent(RuntimeStatus::Done).to_agent_pane().status,
            Some(AgentStatus::Done)
        );
    }

    #[test]
    fn a_local_pane_still_reports_as_having_one() {
        let local = RuntimeAgent {
            reference: AgentRef::new(LOCAL, "%3"),
            status: RuntimeStatus::Working,
            workdir: None,
            title: None,
            kind: None,
        }
        .to_agent_pane();
        // Rendering keeps the bare pane id, but the row is still marked with its
        // runtime, so `has_pane` must reflect the runtime, not the id shape.
        assert_eq!(local.pane_id, "%3");
    }

    #[test]
    fn unsupported_error_names_runtime_and_feature() {
        let e = unsupported("paseo", Feature::Freeze).to_string();
        assert!(e.contains("paseo"), "{e}");
        assert!(e.contains("freeze"), "{e}");
    }
}
