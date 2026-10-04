//! Data structures for the task graph store.

use serde::{Deserialize, Serialize};

/// Subset of `tasks/index.json` task schema needed for orchestration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphTask {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    /// Branch this task's work lives on. Recorded when the tree is created, never
    /// derived from the id: either side may create the worktree and pick the name,
    /// so the name is only knowable by lookup. `None` on records that predate this
    /// field — consumers fall back to the id-derived name and say so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Ref `branch` was cut from: the base branch for a root task, the parent's
    /// branch for a child (which merges back into it, not into base).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// Monotonic dispatch counter, bumped inside the graph lock on every bind. A
    /// settling signal must name the current value or it belongs to a superseded
    /// attempt and is rejected. `None` on pre-upgrade records (legacy gate).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(
        default,
        deserialize_with = "text_or_structured",
        skip_serializing_if = "Option::is_none"
    )]
    pub acceptance_criteria: Option<String>,
    #[serde(
        default,
        deserialize_with = "text_or_structured",
        skip_serializing_if = "Option::is_none"
    )]
    pub agent_hints: Option<String>,
    #[serde(
        default,
        deserialize_with = "text_or_structured",
        skip_serializing_if = "Option::is_none"
    )]
    pub testing_dod: Option<String>,
    /// Implementation plan captured from the planning stage. Persisted back onto
    /// the task by the harness runner (the node flagged `record_plan`) so the plan
    /// survives across stages/restarts and is visible in the task record. The
    /// downstream implement stage receives the same text via `{{outputs.plan}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub implementation_plan: Option<String>,
    /// Free-form labels used by the orchestrate loop for selection filtering and
    /// per-task harness routing (e.g. `backend`, `docs`, `risky`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Optional priority; higher runs first under the `priority` selection order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    /// Setup-phase capability this task provisions (e.g. `test_env`). Tasks with
    /// this set form the setup subgraph: they are run **once** by the
    /// orchestrator preflight (guarded by the capability lock) before any normal
    /// task slots open, and are excluded from the ordinary task frontier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<String>,
    /// Containment edge: the id of the parent task this task was decomposed
    /// FROM. Distinct from `depends_on` (which is intra-level ordering). A task
    /// with `parent` set is a child created by its parent's `decompose` node; it
    /// branches FROM and merges INTO the parent's branch (not `main`). The
    /// parent is a *container* — derived from having children, never declared.
    /// `None` = a root task whose parent is the base branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Frozen exit contract for this task, emitted by the parent's `decompose`
    /// node along the architectural seam. This is a READ-ONLY input flowing
    /// DOWN: a child never re-plans its own contract. It becomes the child's red
    /// acceptance test / definition-of-done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    /// Declared set of symbols/paths this task WRITES (predicted by decompose,
    /// not sniffed at runtime). Pairwise intersection across siblings is a
    /// potential conflict → the decompose reconciler serializes them or inserts
    /// a shared-contract dependency. See `graph::reconcile_conflicts`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub write_set: Vec<String>,
    /// Declared set of symbols/paths this task READS / depends on semantically.
    /// A child's `read_set` intersecting a sibling's `provides` lifts an implicit
    /// invariant into an explicit ordering edge (producer-first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_set: Vec<String>,
    /// Capabilities/contracts this task PRODUCES for siblings to consume. Used
    /// with `read_set` to author producer→consumer ordering during decompose.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provides: Vec<String>,
    // -- OpenSpec provenance (all None/false for ordinary tasks) -------------
    /// Repo-relative path of the `tasks.md` this task was synced from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file: Option<String>,
    /// 0-based line of the item in `source_file` at last sync (write-back hint).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_line: Option<usize>,
    /// Dotted item number as written ("2.1").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_number: Option<String>,
    /// Normalized-text fingerprint of the item at last sync (hex FNV-1a 64).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_fingerprint: Option<String>,
    /// The source item vanished from `tasks.md` while this task was in flight;
    /// the task is retained (its worktree/branch are live) but flagged.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub orphaned: bool,
    // -- Persistent slot/agent state -----------------------------------------
    /// Live execution slot for this task, persisted so the daemon, TUI, and RPC
    /// clients share one durable view and a restarted orchestrator can re-adopt
    /// running harnesses. `None` when no harness is (believed) running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<TaskSlot>,
    /// Unix seconds the task last entered a terminal status (`done`/`failed`),
    /// stamped by [`crate::tasks::graph::update_status`]. Orders "recent
    /// outcomes" in the loop dashboard; `None` for tasks that predate it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    /// Why the task stopped: a stall kill, a harness crash, or the findings of a
    /// failed run. Written by the harness (`task update --blocked-reason-file`)
    /// and by the daemon; read by the failure steward. A value starting with
    /// `steward:` means the steward already ruled on this failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    /// How many times the steward has re-queued this task. Capped at
    /// [`MAX_STEWARD_RETRIES`], after which a failure is parked instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    /// Settling signals that were refused, newest last, capped. Kept as history so
    /// "the worker says it finished but the task did not move" is explainable
    /// instead of silent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected_signals: Vec<RejectedSignal>,
}

/// One refused settling signal.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RejectedSignal {
    /// Unix seconds the signal was refused.
    pub at: u64,
    /// Attempt the signal named, if it named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    /// Machine-readable reason, e.g. `stale_attempt`.
    pub code: String,
}

/// Free-text task fields that real graphs author structurally: a string stays
/// as-is, a list becomes one `- ` line per item, an object becomes its
/// pretty-printed JSON, `null`/missing becomes `None`. Serialization is
/// unchanged (always a string).
fn text_or_structured<'de, D>(de: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    let value = match Option::<Value>::deserialize(de)? {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v,
    };
    Ok(Some(match value {
        Value::String(s) => s,
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::String(s) => format!("- {s}"),
                other => format!("- {other}"),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => serde_json::to_string_pretty(&other).unwrap_or_else(|_| other.to_string()),
    }))
}

/// Persistent record of a claimed execution slot, stored on the task itself.
/// The in-memory `active_slots` map remains the orchestrator's hot-path cache;
/// this is the durable copy that survives daemon restarts and is visible to the
/// task TUI and RPC clients.
/// Where a slot is in the bind → create → run lifecycle.
///
/// The point of `Starting` is that creating a worktree is a side effect outside the
/// graph: a process that dies between recording the intent and confirming the tree
/// leaves state that is neither "running" nor "never started". Without a name for
/// that, recovery has to guess, and guessing wrong either abandons live work or
/// double-creates a tree. `Unknown` is the same admission after the fact: the slot
/// was live, the evidence is gone, and only an explicit decision may settle it.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SlotLifecycle {
    /// Binding recorded, worktree creation not yet confirmed.
    Starting,
    /// Worktree confirmed to exist; the slot is live. Default so pre-upgrade slots,
    /// which only ever existed post-creation, deserialize as confirmed.
    #[default]
    Ready,
    /// Liveness evidence is absent. Never auto-adopted, never auto-released.
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskSlot {
    /// Unix seconds the slot was claimed.
    pub started_at: u64,
    /// Lifecycle position of this slot. Absent in pre-upgrade records, which
    /// deserialize as [`SlotLifecycle::Ready`].
    #[serde(default)]
    pub state: SlotLifecycle,
    /// The task's `attempt` value this slot was bound for. Signals naming a
    /// different attempt are rejected as stale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
    /// Multiplexer pane id running the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_pane: Option<String>,
    /// Multiplexer pane id running the task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness_pane: Option<String>,
    /// Multiplexer window name (e.g. `mx-add-auth-1-2`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<String>,
    /// Agent session id, when known (enables `--resume <session>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Pid of a headless harness process (`orchestrate.harness.mode: headless`).
    /// Persisted so a restarted daemon can re-adopt a live harness instead of
    /// double-spawning it. `None` in pane mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// Patch of mutable [`GraphTask`] fields for an in-place update. A `None` field
/// is left unchanged; `Some` overwrites it. Every other field on disk is
/// preserved. Consumed by [`crate::tasks::graph::update_task`] (and reusable
/// by the gRPC `PatchTask` / TUI edit paths).
#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub worktree: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
    pub priority: Option<i64>,
    /// Appended to `agent_hints` rather than replacing it: retry guidance
    /// accumulates across steward rulings.
    pub hint: Option<String>,
    pub blocked_reason: Option<String>,
    pub retries: Option<u32>,
    /// Labels added / removed, applied after `labels` (if both are given).
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
}

impl TaskPatch {
    /// True if no field is set (so an update would be a no-op).
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.status.is_none()
            && self.worktree.is_none()
            && self.depends_on.is_none()
            && self.labels.is_none()
            && self.priority.is_none()
            && self.hint.is_none()
            && self.blocked_reason.is_none()
            && self.retries.is_none()
            && self.add_labels.is_empty()
            && self.remove_labels.is_empty()
    }
}

/// Label that parks a task: the steward sets it when a failure needs a human,
/// and [`crate::tasks::graph::frontier`] never offers a task carrying it.
pub const NEEDS_HUMAN: &str = "needs-human";
/// Steward re-queues per task before a failure is parked instead.
#[allow(dead_code)]
pub const MAX_STEWARD_RETRIES: u32 = 2;

pub const STATUS_TODO: &str = "todo";
pub const STATUS_IN_PROGRESS: &str = "in_progress";
#[allow(dead_code)]
pub const STATUS_MERGING: &str = "merging";
pub const STATUS_DONE: &str = "done";
pub const STATUS_FAILED: &str = "failed";
/// A container task that has decomposed into children and PARKED its harness,
/// waiting for the orchestrator to wake it once all children are done. The
/// harness process stays alive (agent context warm); the slot is "parked" and
/// does not count against `max_concurrency`. Distinct from `in_progress` so the
/// TUI and slot accounting can tell a working task from a waiting parent.
#[allow(dead_code)]
pub const STATUS_BLOCKED: &str = "blocked";
