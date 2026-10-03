//! Task graph operations: load, frontier computation, atomic status updates, stats.

use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::path::Path;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use super::types::{
    GraphTask, NEEDS_HUMAN, STATUS_DONE, STATUS_FAILED, STATUS_IN_PROGRESS, STATUS_TODO, TaskPatch,
};

/// Read the task graph JSON (an array of tasks).
pub fn load(path: &Path) -> Result<Vec<GraphTask>> {
    let data =
        fs::read_to_string(path).with_context(|| format!("read task graph {}", path.display()))?;
    let tasks: Vec<GraphTask> = serde_json::from_str(&data)
        .with_context(|| format!("parse task graph {}", path.display()))?;
    Ok(tasks)
}

/// True if a task belongs to the setup-phase subgraph (it provisions a
/// capability). Setup tasks are run once by the orchestrator preflight, not the
/// ordinary task frontier.
pub fn is_setup_task(task: &GraphTask) -> bool {
    task.setup.as_deref().is_some_and(|c| !c.is_empty())
}

/// Ordinary (non-setup) tasks with `status == todo` whose dependencies are all
/// `done`. Setup-phase tasks are deliberately excluded — they run during
/// preflight under the capability lock, never as normal task slots. Tasks the
/// steward parked (`needs-human`) are excluded too: the daemon never
/// re-dispatches one until a human drops the label.
pub fn frontier(tasks: &[GraphTask]) -> Vec<&GraphTask> {
    let status_map: HashMap<&str, &str> = tasks
        .iter()
        .map(|t| (t.id.as_str(), t.status.as_str()))
        .collect();

    tasks
        .iter()
        .filter(|t| !is_setup_task(t))
        .filter(|t| t.status == STATUS_TODO)
        .filter(|t| !t.labels.iter().any(|l| l == NEEDS_HUMAN))
        .filter(|t| deps_done(t, &status_map))
        .collect()
}

fn deps_done(task: &GraphTask, status_map: &HashMap<&str, &str>) -> bool {
    task.depends_on
        .iter()
        .all(|dep| status_map.get(dep.as_str()) == Some(&STATUS_DONE))
}


/// Distinct capabilities declared by the setup-phase subgraph, in
/// first-appearance order. This is the set the preflight iterates over.
pub fn setup_capabilities(tasks: &[GraphTask]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for t in tasks {
        if let Some(cap) = t.setup.as_deref().filter(|c| !c.is_empty())
            && seen.insert(cap.to_string())
        {
            out.push(cap.to_string());
        }
    }
    out
}

/// Setup tasks for `capability` with `status == todo` whose dependencies are all
/// `done`, in graph order. Used by the preflight owner to run a capability's
/// subgraph step by step.
pub fn setup_frontier<'a>(tasks: &'a [GraphTask], capability: &str) -> Vec<&'a GraphTask> {
    let status_map: HashMap<&str, &str> = tasks
        .iter()
        .map(|t| (t.id.as_str(), t.status.as_str()))
        .collect();
    tasks
        .iter()
        .filter(|t| t.setup.as_deref() == Some(capability))
        .filter(|t| t.status == STATUS_TODO)
        .filter(|t| deps_done(t, &status_map))
        .collect()
}

/// True if every setup task for `capability` has reached `done`. Returns false
/// if the capability has no setup tasks (nothing to be done means it cannot be
/// considered provisioned by the graph).
pub fn capability_tasks_done(tasks: &[GraphTask], capability: &str) -> bool {
    let mut any = false;
    for t in tasks
        .iter()
        .filter(|t| t.setup.as_deref() == Some(capability))
    {
        any = true;
        if t.status != STATUS_DONE {
            return false;
        }
    }
    any
}

/// True if any setup task for `capability` is in a `failed` state.
pub fn capability_tasks_failed(tasks: &[GraphTask], capability: &str) -> bool {
    tasks
        .iter()
        .filter(|t| t.setup.as_deref() == Some(capability))
        .any(|t| t.status == STATUS_FAILED)
}

// ---------------------------------------------------------------------------
// Containment (parent/children) — the recursive-decomposition edge, distinct
// from `depends_on` (ordering). A *container* is any task that has children;
// it is never declared, only derived. See `docs/recursive-decomposition.md`.
// ---------------------------------------------------------------------------

/// All tasks whose `parent` points at `parent_id`, in graph order.
pub fn children_of<'a>(tasks: &'a [GraphTask], parent_id: &str) -> Vec<&'a GraphTask> {
    tasks
        .iter()
        .filter(|t| t.parent.as_deref() == Some(parent_id))
        .collect()
}

/// True if `id` has at least one child task (i.e. it decomposed → it is a
/// container). Derived, never declared.
pub fn is_container(tasks: &[GraphTask], id: &str) -> bool {
    tasks.iter().any(|t| t.parent.as_deref() == Some(id))
}

/// True if `parent_id` has children and **every** child has reached `done`.
/// Returns false when the task has no children (nothing to join). This is the
/// orchestrator's wake condition for a parked container.
pub fn all_children_done(tasks: &[GraphTask], parent_id: &str) -> bool {
    let mut any = false;
    for c in tasks
        .iter()
        .filter(|t| t.parent.as_deref() == Some(parent_id))
    {
        any = true;
        if c.status != STATUS_DONE {
            return false;
        }
    }
    any
}

/// True if any child of `parent_id` has failed — the join cannot proceed and the
/// container should surface the failure rather than wake into a broken union.
pub fn any_child_failed(tasks: &[GraphTask], parent_id: &str) -> bool {
    tasks
        .iter()
        .filter(|t| t.parent.as_deref() == Some(parent_id))
        .any(|c| c.status == STATUS_FAILED)
}

/// Persist a child task's containment + contract fields. Used by the decompose
/// node when materializing children (it builds the full `GraphTask` and calls
/// [`add_task`]); this is the in-place variant for tests/repairs.
pub fn set_parent(path: &Path, task_id: &str, parent_id: &str) -> Result<()> {
    let parent = parent_id.to_string();
    mutate_task_locked(path, task_id, move |task| {
        task["parent"] = Value::String(parent.clone());
    })
}

/// Author conflict-preventing ordering across a freshly-decomposed sibling set,
/// IN PLACE, returning a human-readable note for each edge it added.
///
/// Two passes, both declarative (predicted sets, never runtime sniffing):
///
/// 1. **Producer → consumer.** If child B's `read_set` intersects child A's
///    `provides`, B must see A's work first → add `B.depends_on += A`. This lifts
///    an implicit semantic invariant into an explicit ordering edge.
/// 2. **Write/write disjointness.** If two children's `write_set`s intersect and
///    neither already (transitively) depends on the other, they would race on the
///    same symbols → serialize them deterministically (later-in-order depends on
///    earlier) so their write-sets become effectively disjoint in time.
///
/// Children are matched by `id`; edges are only added between members of `ids`.
/// The function is order-stable and idempotent (re-running adds nothing new).
pub fn reconcile_conflicts(children: &mut [GraphTask]) -> Vec<String> {
    let mut notes = Vec::new();
    let ids: HashSet<String> = children.iter().map(|c| c.id.clone()).collect();

    // Snapshot the declared sets so we can read A while mutating B.
    let provides: HashMap<String, HashSet<String>> = children
        .iter()
        .map(|c| (c.id.clone(), c.provides.iter().cloned().collect()))
        .collect();
    let write_sets: HashMap<String, HashSet<String>> = children
        .iter()
        .map(|c| (c.id.clone(), c.write_set.iter().cloned().collect()))
        .collect();
    let order: Vec<String> = children.iter().map(|c| c.id.clone()).collect();
    let pos: HashMap<&str, usize> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();

    // Pass 1: producer → consumer via read_set ∩ provides.
    for consumer in children.iter_mut() {
        let reads: HashSet<&str> = consumer.read_set.iter().map(|s| s.as_str()).collect();
        for (producer_id, prod) in &provides {
            if producer_id == &consumer.id {
                continue;
            }
            if prod.iter().any(|p| reads.contains(p.as_str()))
                && !consumer.depends_on.contains(producer_id)
            {
                consumer.depends_on.push(producer_id.clone());
                notes.push(format!(
                    "ordering: `{}` reads what `{}` provides → producer-first",
                    consumer.id, producer_id
                ));
            }
        }
    }

    // Pass 2: serialize write/write overlaps that aren't already ordered.
    // Compare each unordered pair once (i < j in graph order); the later one
    // gains a dependency on the earlier so their writes can't interleave.
    for i in 0..order.len() {
        for j in (i + 1)..order.len() {
            let (a, b) = (&order[i], &order[j]);
            let (Some(wa), Some(wb)) = (write_sets.get(a), write_sets.get(b)) else {
                continue;
            };
            let overlap: Vec<&str> = wa
                .iter()
                .filter(|s| wb.contains(s.as_str()))
                .map(|s| s.as_str())
                .collect();
            if overlap.is_empty() {
                continue;
            }
            if !ids.contains(a) || !ids.contains(b) {
                continue;
            }
            // Already ordered either direction? then the race is resolved.
            let b_idx = pos[b.as_str()];
            let already = {
                let bn = &children[b_idx];
                bn.depends_on.contains(a)
            } || {
                let a_idx = pos[a.as_str()];
                children[a_idx].depends_on.contains(b)
            };
            if already {
                continue;
            }
            children[b_idx].depends_on.push(a.clone());
            notes.push(format!(
                "serialize: `{}` and `{}` both write {{{}}} → `{}` after `{}`",
                a,
                b,
                overlap.join(", "),
                b,
                a
            ));
        }
    }

    notes
}

/// Atomically update a single task's status in the graph JSON.
///
/// Uses a `.lock` sidecar file (created exclusively, retried 10x with 150ms
/// backoff) plus a tmp-write + rename so concurrent writers never clobber each
/// other. All other fields are preserved by editing the parsed `serde_json::Value`.
pub fn update_status(path: &Path, task_id: &str, new_status: &str) -> Result<()> {
    let status = new_status.to_string();
    mutate_task_locked(path, task_id, move |task| {
        // Stamp terminal transitions so observers (loop dashboard) can order
        // recent outcomes without a separate event store.
        if matches!(status.as_str(), STATUS_DONE | STATUS_FAILED) {
            task["completed_at"] = Value::from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
        }
        task["status"] = Value::String(status.clone());
    })
}

/// Persist the planning stage's output onto the task as `implementation_plan`,
/// so the plan survives across stages/restarts and is visible in the task record.
pub fn set_implementation_plan(path: &Path, task_id: &str, plan: &str) -> Result<()> {
    let plan = plan.to_string();
    mutate_task_locked(path, task_id, move |task| {
        task["implementation_plan"] = Value::String(plan.clone());
    })
}

/// Record why a task stopped (stall kill, harness crash, findings from a failed
/// run) so the TUI and the failure steward can read it off the task record.
pub fn set_blocked_reason(path: &Path, task_id: &str, reason: &str) -> Result<()> {
    let reason = reason.to_string();
    mutate_task_locked(path, task_id, move |task| {
        task["blocked_reason"] = Value::String(reason.clone());
    })
}

/// Acquire the graph's sidecar lock, load the JSON array, apply `mutate` to the
/// task matching `task_id`, then atomically write the graph back. The lock is
/// always released, regardless of how we exit.
fn mutate_task_locked<F>(path: &Path, task_id: &str, mutate: F) -> Result<()>
where
    F: FnOnce(&mut Value),
{
    with_lock(path, || mutate_task_inner(path, task_id, mutate))
}

fn mutate_task_inner<F>(path: &Path, task_id: &str, mutate: F) -> Result<()>
where
    F: FnOnce(&mut Value),
{
    let data =
        fs::read_to_string(path).with_context(|| format!("read task graph {}", path.display()))?;
    let mut value: Value = serde_json::from_str(&data)
        .with_context(|| format!("parse task graph {}", path.display()))?;

    let tasks = value
        .as_array_mut()
        .ok_or_else(|| anyhow!("task graph {} is not a JSON array", path.display()))?;

    let mut mutate = Some(mutate);
    let mut found = false;
    for task in tasks.iter_mut() {
        if task.get("id").and_then(Value::as_str) == Some(task_id) {
            if let Some(f) = mutate.take() {
                f(task);
            }
            found = true;
            break;
        }
    }
    if !found {
        return Err(anyhow!("task {:?} not found", task_id));
    }

    let out = serde_json::to_string_pretty(&value).context("serialize task graph")?;

    let tmp = path.with_extension(tmp_extension(path));
    fs::write(&tmp, out).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Atomically append a new task to the graph JSON array.
///
/// Fails if a task with the same id already exists.
pub fn add_task(path: &Path, task: &GraphTask) -> Result<()> {
    with_lock(path, || add_task_locked(path, task, None))
}

/// Fields a child never inherits from its parent: identity, lifecycle state and
/// provenance. Everything else on the parent record — including fields this
/// binary does not model, such as `harness_layers` — flows down.
const NOT_INHERITED: &[&str] = &[
    "id",
    "title",
    "description",
    "status",
    "depends_on",
    "worktree",
    "branch",
    "base",
    "attempt",
    "labels",
    "parent",
    "slot",
    "setup",
    "completed_at",
    "blocked_reason",
    "retries",
    "implementation_plan",
    "orphaned",
    "source_file",
    "source_line",
    "source_number",
    "source_fingerprint",
];

/// [`add_task`], but every field of `inherit_from` that the new task does not
/// set itself (and that is not in [`NOT_INHERITED`]) is copied onto it. Used by
/// `task create --inherit-from` so split children carry the parent's harness
/// and testing configuration.
pub fn add_task_inheriting(path: &Path, task: &GraphTask, inherit_from: &str) -> Result<()> {
    with_lock(path, || add_task_locked(path, task, Some(inherit_from)))
}

fn add_task_locked(path: &Path, task: &GraphTask, inherit_from: Option<&str>) -> Result<()> {
    let data =
        fs::read_to_string(path).with_context(|| format!("read task graph {}", path.display()))?;
    let mut value: Value = serde_json::from_str(&data)
        .with_context(|| format!("parse task graph {}", path.display()))?;
    let tasks = value
        .as_array_mut()
        .ok_or_else(|| anyhow!("task graph is not a JSON array"))?;

    if tasks
        .iter()
        .any(|t| t.get("id").and_then(Value::as_str) == Some(&task.id))
    {
        return Err(anyhow!("task {:?} already exists", task.id));
    }

    let mut new_task = serde_json::to_value(task).context("serialize task")?;
    if let Some(src_id) = inherit_from {
        let src = tasks
            .iter()
            .find(|t| t.get("id").and_then(Value::as_str) == Some(src_id))
            .cloned()
            .ok_or_else(|| anyhow!("inherit-from task {src_id:?} not found"))?;
        let (Some(src), Some(dst)) = (src.as_object(), new_task.as_object_mut()) else {
            return Err(anyhow!("task records are not JSON objects"));
        };
        for (k, v) in src {
            if !NOT_INHERITED.contains(&k.as_str()) && !dst.contains_key(k) {
                dst.insert(k.clone(), v.clone());
            }
        }
    }
    tasks.push(new_task);

    let out = serde_json::to_string_pretty(&value).context("serialize task graph")?;
    let tmp = path.with_extension(tmp_extension(path));
    fs::write(&tmp, out).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Atomically apply a field patch to an existing task, preserving every field
/// not named in the patch (including ones this binary doesn't model).
pub fn update_task(path: &Path, task_id: &str, patch: &TaskPatch) -> Result<()> {
    mutate_task_locked(path, task_id, |task| {
        let str_arr =
            |v: &[String]| Value::Array(v.iter().map(|s| Value::String(s.clone())).collect());
        if let Some(v) = &patch.title {
            task["title"] = Value::String(v.clone());
        }
        if let Some(v) = &patch.description {
            task["description"] = Value::String(v.clone());
        }
        if let Some(v) = &patch.status {
            task["status"] = Value::String(v.clone());
        }
        if let Some(v) = &patch.worktree {
            task["worktree"] = Value::String(v.clone());
        }
        if let Some(v) = &patch.depends_on {
            task["depends_on"] = str_arr(v);
        }
        if let Some(v) = &patch.labels {
            task["labels"] = str_arr(v);
        }
        if let Some(v) = &patch.priority {
            task["priority"] = Value::Number((*v).into());
        }
        if let Some(v) = &patch.blocked_reason {
            task["blocked_reason"] = Value::String(v.clone());
        }
        if let Some(v) = &patch.retries {
            task["retries"] = Value::Number((*v).into());
        }
        if let Some(hint) = &patch.hint {
            // Append, preserving whatever shape the field already has on disk
            // (real graphs author `agent_hints` as a string or a list).
            match task.get_mut("agent_hints") {
                Some(Value::Array(items)) => items.push(Value::String(hint.clone())),
                Some(Value::String(s)) if !s.is_empty() => {
                    let joined = format!("{s}\n{hint}");
                    task["agent_hints"] = Value::String(joined);
                }
                _ => task["agent_hints"] = Value::String(hint.clone()),
            }
        }
        if !patch.add_labels.is_empty() || !patch.remove_labels.is_empty() {
            let mut labels: Vec<String> = task
                .get("labels")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            for l in &patch.add_labels {
                if !labels.contains(l) {
                    labels.push(l.clone());
                }
            }
            labels.retain(|l| !patch.remove_labels.contains(l));
            task["labels"] = str_arr(&labels);
        }
    })
}

/// Persist a claimed execution slot onto a task (see [`TaskSlot`]).
pub fn set_slot(path: &Path, task_id: &str, slot: &crate::tasks::types::TaskSlot) -> Result<()> {
    let value = serde_json::to_value(slot).context("serialize task slot")?;
    mutate_task_locked(path, task_id, move |task| {
        task["slot"] = value;
    })
}

/// What [`bind_slot`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindOutcome {
    /// A fresh binding was recorded; the task now carries this attempt number.
    Bound { attempt: u32 },
    /// An identical binding was already present. Returned instead of an error so a
    /// claim retried after a lost response is safe to repeat.
    AlreadyBound { attempt: u32 },
    /// The task is already bound to something else, and that binding is unsettled.
    Conflict {
        branch: Option<String>,
        worktree: Option<String>,
        attempt: Option<u32>,
    },
}

/// Record the intent to run `task_id` in `worktree_path` on `branch` (cut from
/// `base`), bumping the task's attempt counter, in one locked write.
///
/// Called BEFORE the worktree is created, by whichever side creates it. The slot
/// lands in `starting`; [`confirm_slot`] promotes it once the tree exists. A crash
/// in between is then readable as "may or may not have been created" rather than
/// being indistinguishable from a task that never started.
pub fn bind_slot(
    path: &Path,
    task_id: &str,
    branch: &str,
    base: &str,
    worktree_path: &str,
) -> Result<BindOutcome> {
    let mut outcome = None;
    edit_graph(path, |tasks| {
        let task = tasks
            .iter_mut()
            .find(|t| t["id"] == task_id)
            .ok_or_else(|| anyhow!("task {task_id:?} not found"))?;

        // An unsettled slot owns the task. `unknown` counts as unsettled on purpose:
        // its worktree may still exist, so handing the task to a second binding is
        // exactly the divergence this mechanism exists to prevent.
        if let Some(slot) = task.get("slot").filter(|s| !s.is_null()).cloned() {
            let attempt = slot["attempt"].as_u64().map(|a| a as u32);
            let same = task["branch"] == Value::from(branch)
                && task["base"] == Value::from(base)
                && slot["worktree_path"] == Value::from(worktree_path);
            outcome = Some(if same {
                BindOutcome::AlreadyBound {
                    attempt: attempt.unwrap_or_default(),
                }
            } else {
                BindOutcome::Conflict {
                    branch: task["branch"].as_str().map(str::to_string),
                    worktree: slot["worktree_path"].as_str().map(str::to_string),
                    attempt,
                }
            });
            return Ok(());
        }

        let attempt = task["attempt"].as_u64().unwrap_or(0) as u32 + 1;
        task["branch"] = Value::from(branch);
        task["base"] = Value::from(base);
        task["attempt"] = Value::from(attempt);
        task["slot"] = serde_json::json!({
            "started_at": now_secs(),
            "worktree_path": worktree_path,
            "state": "starting",
            "attempt": attempt,
        });
        outcome = Some(BindOutcome::Bound { attempt });
        Ok(())
    })?;
    outcome.ok_or_else(|| anyhow!("bind_slot produced no outcome for {task_id:?}"))
}

/// Promote a `starting` slot to `ready` once its worktree is confirmed to exist.
/// Returns whether the promotion happened.
///
/// A mismatched attempt is a no-op: that confirmation belongs to a binding that has
/// since been superseded, and applying it would mark the current attempt confirmed
/// on the strength of an older one's side effect.
pub fn confirm_slot(path: &Path, task_id: &str, attempt: u32) -> Result<bool> {
    let mut confirmed = false;
    mutate_task_locked(path, task_id, |task| {
        if task["slot"]["attempt"].as_u64() == Some(attempt as u64) {
            task["slot"]["state"] = Value::from("ready");
            confirmed = true;
        }
    })?;
    Ok(confirmed)
}

/// Mark a slot `unknown`: it was live, and the evidence for that is now gone.
///
/// Deliberately does NOT touch the task's status or clear the slot. The worktree
/// may still hold work and the worker may still be alive, so only an explicit
/// `task resolve` may settle it. Releasing here is what used to race a live worker
/// against a re-dispatch of the same task.
pub fn mark_slot_unknown(path: &Path, task_id: &str) -> Result<()> {
    mutate_task_locked(path, task_id, |task| {
        if !task["slot"].is_null() {
            task["slot"]["state"] = Value::from("unknown");
        }
    })
}


fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Release a task's persistent slot. No-op (still Ok) if the task has none;
/// errors only if the task itself is missing.
pub fn clear_slot(path: &Path, task_id: &str) -> Result<()> {
    mutate_task_locked(path, task_id, |task| {
        if let Some(obj) = task.as_object_mut() {
            obj.remove("slot");
        }
    })
}

/// Clear a task's slot and set its resulting status in ONE locked write.
///
/// The pair used to be two calls. A crash between them left a task with no slot but
/// a running status: invisible to the frontier because it is not `todo`, and
/// invisible to slot accounting because it has no slot. Nothing reclaimed it.
pub fn release_slot(path: &Path, task_id: &str, new_status: &str) -> Result<()> {
    let status = new_status.to_string();
    mutate_task_locked(path, task_id, move |task| {
        if let Some(obj) = task.as_object_mut() {
            obj.remove("slot");
        }
        if matches!(status.as_str(), STATUS_DONE | STATUS_FAILED) {
            task["completed_at"] = Value::from(now_secs());
        }
        task["status"] = Value::String(status.clone());
    })
}

/// Why a settling signal was refused. Mirrors the subset of orca's lifecycle
/// rejection codes that apply to a single-host harness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectionCode {
    /// The signal named no attempt, and the slot has one to name.
    MissingAttempt,
    /// The signal named an attempt that is no longer current.
    StaleAttempt,
    /// The caller is not the worker the current attempt was dispatched to.
    NotBoundWorker,
    /// The current attempt already settled this task.
    Duplicate,
    /// No task with that id is in the graph.
    UnknownTask,
}

impl RejectionCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingAttempt => "missing_attempt",
            Self::StaleAttempt => "stale_attempt",
            Self::NotBoundWorker => "not_bound_worker",
            Self::Duplicate => "duplicate",
            Self::UnknownTask => "unknown_task",
        }
    }
}

/// How many rejected signals a task keeps. Enough to explain a confused worker,
/// bounded so a signal loop cannot grow the graph without limit.
const MAX_REJECTED_SIGNALS: usize = 10;

/// Record a refused settling signal on the task as history, oldest dropped past the
/// cap. A rejection that is merely dropped leaves no way to explain why a worker
/// believed it finished and the task did not move.
pub fn record_rejected_signal(
    path: &Path,
    task_id: &str,
    attempt: Option<u32>,
    code: RejectionCode,
) -> Result<()> {
    mutate_task_locked(path, task_id, move |task| {
        let entry = serde_json::json!({
            "at": now_secs(),
            "attempt": attempt,
            "code": code.as_str(),
        });
        let log = task["rejected_signals"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let keep = log.len().saturating_sub(MAX_REJECTED_SIGNALS - 1);
        let mut log: Vec<Value> = log.into_iter().skip(keep).collect();
        log.push(entry);
        task["rejected_signals"] = Value::Array(log);
    })
}

/// Bulk-edit the graph under one lock + one atomic rename. `f` receives the
/// parsed JSON array; unknown fields on tasks are preserved (Value-level edit).
/// This is the primitive OpenSpec sync reconciles through: one lock per
/// tasks.md, not one per touched task.
pub fn edit_graph<F>(path: &Path, f: F) -> Result<()>
where
    F: FnOnce(&mut Vec<Value>) -> Result<()>,
{
    with_lock(path, || {
        let data = fs::read_to_string(path)
            .with_context(|| format!("read task graph {}", path.display()))?;
        let mut value: Value = serde_json::from_str(&data)
            .with_context(|| format!("parse task graph {}", path.display()))?;
        let tasks = value
            .as_array_mut()
            .ok_or_else(|| anyhow!("task graph {} is not a JSON array", path.display()))?;
        f(tasks)?;
        let out = serde_json::to_string_pretty(&value).context("serialize task graph")?;
        let tmp = path.with_extension(tmp_extension(path));
        fs::write(&tmp, out).with_context(|| format!("write {}", tmp.display()))?;
        fs::rename(&tmp, path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
        Ok(())
    })
}

/// How long a lock may sit before a contender takes it regardless of what the
/// recorded pid says. Overridable so tests do not have to sleep.
fn lock_stale_secs() -> u64 {
    std::env::var("WORKMUX_GRAPH_LOCK_STALE_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
}

/// True if `pid` still names a live process.
///
/// Pid 0 is rejected up front: `kill(0, 0)` signals the caller's whole process
/// group and reports success, so a lock naming pid 0 would look eternally alive.
fn process_alive(pid: u32) -> bool {
    pid != 0 && unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

/// The pid recorded in a lock that may be reclaimed — holder gone, or lock older
/// than the staleness bound. `None` means leave it alone and keep waiting.
///
/// A lock whose contents do not parse is a legacy (empty) lock from a binary that
/// predates this format; it is judged on file age alone, since it is exactly the
/// case that otherwise blocks the graph forever.
fn reclaimable_lock_holder(lock_path: &Path) -> Option<u32> {
    let contents = fs::read_to_string(lock_path).ok()?;
    let mut parts = contents.split_whitespace();
    let parsed = parts
        .next()
        .and_then(|p| p.parse::<u32>().ok())
        .zip(parts.next().and_then(|t| t.parse::<u64>().ok()));

    let Some((pid, since)) = parsed else {
        let age = fs::metadata(lock_path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|m| m.elapsed().ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        return (age > lock_stale_secs()).then_some(0);
    };

    let expired = now_secs().saturating_sub(since) > lock_stale_secs();
    (expired || !process_alive(pid)).then_some(pid)
}

/// Acquire the graph's sidecar lock, run `f`, then always release the lock.
///
/// The lock file holds `<pid> <unix_seconds>`. A contender reclaims it when the
/// holder is gone or the lock has outlived the staleness bound. Without that, a
/// writer killed mid-write leaves an empty lock that no later writer can clear, and
/// every subsequent graph write fails — the graph is bricked until someone deletes
/// the file by hand.
fn with_lock<T>(path: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    use std::io::Write as _;

    let lock_path = path.with_extension(lock_extension(path));
    let mut acquired = false;
    for _ in 0..10 {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
        {
            Ok(mut file) => {
                let _ = write!(file, "{} {}", std::process::id(), now_secs());
                acquired = true;
                break;
            }
            Err(_) => match reclaimable_lock_holder(&lock_path) {
                Some(holder) => {
                    crate::wm_evt!(
                        "graph.lock.reclaim",
                        lock = %lock_path.display(),
                        holder = holder
                    );
                    let _ = fs::remove_file(&lock_path);
                }
                None => thread::sleep(Duration::from_millis(150)),
            },
        }
    }
    if !acquired {
        return Err(anyhow!("acquire lock {}", lock_path.display()));
    }
    let result = f();
    let _ = fs::remove_file(&lock_path);
    result
}

/// Atomically remove a task by id. Errors if no such task exists. Note: this
/// does not rewrite `depends_on` edges on other tasks that referenced the
/// deleted id — the caller is responsible for fixing dangling deps.
pub fn delete_task(path: &Path, task_id: &str) -> Result<()> {
    with_lock(path, || {
        let data = fs::read_to_string(path)
            .with_context(|| format!("read task graph {}", path.display()))?;
        let mut value: Value = serde_json::from_str(&data)
            .with_context(|| format!("parse task graph {}", path.display()))?;
        let tasks = value
            .as_array_mut()
            .ok_or_else(|| anyhow!("task graph is not a JSON array"))?;
        let before = tasks.len();
        tasks.retain(|t| t.get("id").and_then(Value::as_str) != Some(task_id));
        if tasks.len() == before {
            return Err(anyhow!("task {:?} not found", task_id));
        }
        let out = serde_json::to_string_pretty(&value).context("serialize task graph")?;
        let tmp = path.with_extension(tmp_extension(path));
        fs::write(&tmp, out).with_context(|| format!("write {}", tmp.display()))?;
        fs::rename(&tmp, path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
        Ok(())
    })
}

/// Returns `(total, done, in_progress, failed)`.
pub fn stats(tasks: &[GraphTask]) -> (usize, usize, usize, usize) {
    let mut done = 0;
    let mut in_progress = 0;
    let mut failed = 0;
    for t in tasks {
        match t.status.as_str() {
            STATUS_DONE => done += 1,
            STATUS_IN_PROGRESS => in_progress += 1,
            STATUS_FAILED => failed += 1,
            _ => {}
        }
    }
    (tasks.len(), done, in_progress, failed)
}

/// Count of tasks with `status == todo` (used for the `graph --all` summary).
pub fn todo_count(tasks: &[GraphTask]) -> usize {
    tasks.iter().filter(|t| t.status == STATUS_TODO).count()
}

/// Append the lock suffix to whatever extension the graph file already has, so
/// `tasks/index.json` becomes `tasks/index.json.lock`.
fn lock_extension(path: &Path) -> String {
    suffixed_extension(path, "lock")
}

fn tmp_extension(path: &Path) -> String {
    suffixed_extension(path, "tmp")
}

fn suffixed_extension(path: &Path, suffix: &str) -> String {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.{suffix}"),
        None => suffix.to_string(),
    }
}

/// Set of task IDs that are unblocked, used to avoid double-spawning.
#[allow(dead_code)]
pub fn frontier_ids(tasks: &[GraphTask]) -> HashSet<String> {
    frontier(tasks).into_iter().map(|t| t.id.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::STATUS_BLOCKED;

    fn task(id: &str, status: &str, deps: &[&str]) -> GraphTask {
        GraphTask {
            id: id.to_string(),
            title: format!("Title {id}"),
            description: String::new(),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            status: status.to_string(),
            ..Default::default()
        }
    }

    fn setup_task(id: &str, status: &str, cap: &str, deps: &[&str]) -> GraphTask {
        GraphTask {
            setup: Some(cap.to_string()),
            ..task(id, status, deps)
        }
    }

    fn labeled(id: &str, labels: &[&str], priority: Option<i64>) -> GraphTask {
        GraphTask {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            priority,
            ..task(id, STATUS_TODO, &[])
        }
    }





    #[test]
    fn set_implementation_plan_persists_and_preserves_other_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        let tasks = vec![task("a", STATUS_TODO, &[]), task("b", STATUS_TODO, &["a"])];
        fs::write(&path, serde_json::to_string_pretty(&tasks).unwrap()).unwrap();

        set_implementation_plan(&path, "a", "step 1\nstep 2").unwrap();

        let loaded = load(&path).unwrap();
        let a = loaded.iter().find(|t| t.id == "a").unwrap();
        assert_eq!(a.implementation_plan.as_deref(), Some("step 1\nstep 2"));
        // Untouched task keeps no plan and its status.
        let b = loaded.iter().find(|t| t.id == "b").unwrap();
        assert!(b.implementation_plan.is_none());
        assert_eq!(b.status, STATUS_TODO);
    }

    #[test]
    fn set_implementation_plan_unknown_task_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        fs::write(
            &path,
            serde_json::to_string(&vec![task("a", STATUS_TODO, &[])]).unwrap(),
        )
        .unwrap();
        assert!(set_implementation_plan(&path, "missing", "x").is_err());
    }

    #[test]
    fn frontier_returns_unblocked_todo_tasks() {
        let tasks = vec![
            task("a", STATUS_DONE, &[]),
            task("b", STATUS_TODO, &["a"]),
            task("c", STATUS_TODO, &["a"]),
            task("d", STATUS_TODO, &["b"]),
        ];
        let frontier_ids: Vec<&str> = frontier(&tasks).iter().map(|t| t.id.as_str()).collect();
        assert_eq!(frontier_ids, vec!["b", "c"]);
    }

    #[test]
    fn frontier_excludes_parked_tasks() {
        // A `needs-human` task is never re-offered, however ready it looks.
        let tasks = vec![
            labeled("parked", &[NEEDS_HUMAN], None),
            labeled("live", &["backend"], None),
        ];
        let ids: Vec<&str> = frontier(&tasks).iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["live"]);
    }

    #[test]
    fn update_task_appends_hint_and_edits_labels() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        let raw = serde_json::json!([{
            "id": "a",
            "title": "Title a",
            "status": STATUS_FAILED,
            "agent_hints": "first hint",
            "labels": ["backend"],
        }, {
            "id": "b",
            "title": "Title b",
            "status": STATUS_TODO,
            "agent_hints": ["listed hint"],
        }]);
        fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        update_task(
            &path,
            "a",
            &TaskPatch {
                status: Some(STATUS_TODO.into()),
                hint: Some("second hint".into()),
                blocked_reason: Some("steward: retry 1/2".into()),
                retries: Some(1),
                add_labels: vec!["retried".into(), "backend".into()],
                remove_labels: vec![NEEDS_HUMAN.into()],
                ..Default::default()
            },
        )
        .unwrap();
        // A list-shaped agent_hints keeps its shape: the hint is appended as an item.
        update_task(
            &path,
            "b",
            &TaskPatch {
                hint: Some("another".into()),
                add_labels: vec![NEEDS_HUMAN.into()],
                ..Default::default()
            },
        )
        .unwrap();

        let loaded = load(&path).unwrap();
        let a = loaded.iter().find(|t| t.id == "a").unwrap();
        assert_eq!(a.status, STATUS_TODO);
        assert_eq!(a.agent_hints.as_deref(), Some("first hint\nsecond hint"));
        assert_eq!(a.blocked_reason.as_deref(), Some("steward: retry 1/2"));
        assert_eq!(a.retries, Some(1));
        // Existing label kept, new one added once, absent removal is a no-op.
        assert_eq!(a.labels, vec!["backend".to_string(), "retried".to_string()]);

        let b = loaded.iter().find(|t| t.id == "b").unwrap();
        assert_eq!(b.agent_hints.as_deref(), Some("- listed hint\n- another"));
        assert_eq!(b.labels, vec![NEEDS_HUMAN.to_string()]);
        // Parked: no longer on the frontier.
        assert!(!frontier(&loaded).iter().any(|t| t.id == "b"));
    }

    #[test]
    fn add_task_inheriting_copies_unmodelled_parent_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        let raw = serde_json::json!([{
            "id": "parent",
            "title": "Parent",
            "status": STATUS_FAILED,
            "labels": [NEEDS_HUMAN],
            "testing_dod": "make test",
            "harness_layers": {"replay": true},
            "blocked_reason": "steward: split into child",
            "branch": "feat/parent-work",
            "base": "main",
            "attempt": 3,
        }]);
        fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        let child = GraphTask {
            parent: Some("parent".into()),
            depends_on: vec!["parent-sibling".into()],
            ..task("child", STATUS_TODO, &[])
        };
        add_task_inheriting(&path, &child, "parent").unwrap();

        let value: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let c = value
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == "child")
            .unwrap();
        assert_eq!(c["harness_layers"], serde_json::json!({"replay": true}));
        assert_eq!(c["testing_dod"], "make test");
        assert_eq!(c["parent"], "parent");
        assert_eq!(c["status"], STATUS_TODO);
        // Identity and lifecycle state never flow down.
        assert_eq!(c["title"], "Title child");
        assert!(c.get("blocked_reason").is_none());
        assert!(c.get("labels").is_none(), "parked label must not inherit");
        // A child cuts its own branch from the parent's and is dispatched on its own
        // counter: inheriting either would bind two tasks to one tree.
        for owned in ["branch", "base", "attempt"] {
            assert!(c.get(owned).is_none(), "{owned} must not inherit");
        }
        assert_eq!(c["depends_on"], serde_json::json!(["parent-sibling"]));

        assert!(add_task_inheriting(&path, &task("x", STATUS_TODO, &[]), "ghost").is_err());
    }

    fn graph_with(tasks: &[GraphTask]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        fs::write(&path, serde_json::to_string_pretty(&tasks).unwrap()).unwrap();
        (dir, path)
    }

    fn slot_of(path: &Path, id: &str) -> Value {
        let value: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        value
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == id)
            .unwrap()
            .clone()
    }

    #[test]
    fn bind_slot_records_a_starting_binding_and_bumps_the_attempt() {
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);

        let outcome = bind_slot(&path, "a", "feat/login", "main", "/wt/login").unwrap();

        assert_eq!(outcome, BindOutcome::Bound { attempt: 1 });
        let t = slot_of(&path, "a");
        assert_eq!(t["branch"], "feat/login");
        assert_eq!(t["base"], "main");
        assert_eq!(t["attempt"], 1);
        // Starting, not ready: the worktree does not exist yet.
        assert_eq!(t["slot"]["state"], "starting");
        assert_eq!(t["slot"]["attempt"], 1);
    }

    #[test]
    fn bind_slot_is_idempotent_for_an_identical_rebind() {
        // A claim retried after a lost response must not burn a second attempt
        // number, or the first response's attempt is instantly stale.
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);
        bind_slot(&path, "a", "feat/login", "main", "/wt/login").unwrap();

        let again = bind_slot(&path, "a", "feat/login", "main", "/wt/login").unwrap();

        assert_eq!(again, BindOutcome::AlreadyBound { attempt: 1 });
        assert_eq!(slot_of(&path, "a")["attempt"], 1);
    }

    #[test]
    fn bind_slot_rejects_a_different_binding_while_one_is_unsettled() {
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);
        bind_slot(&path, "a", "feat/login", "main", "/wt/login").unwrap();

        let clash = bind_slot(&path, "a", "feat/other", "main", "/wt/other").unwrap();

        assert_eq!(
            clash,
            BindOutcome::Conflict {
                branch: Some("feat/login".into()),
                worktree: Some("/wt/login".into()),
                attempt: Some(1),
            }
        );
        // The existing binding is untouched by the rejected claim.
        let t = slot_of(&path, "a");
        assert_eq!(t["branch"], "feat/login");
        assert_eq!(t["attempt"], 1);
    }

    #[test]
    fn confirm_slot_promotes_only_the_matching_attempt() {
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);
        bind_slot(&path, "a", "feat/login", "main", "/wt/login").unwrap();

        assert!(!confirm_slot(&path, "a", 99).unwrap());
        assert_eq!(slot_of(&path, "a")["slot"]["state"], "starting");

        assert!(confirm_slot(&path, "a", 1).unwrap());
        assert_eq!(slot_of(&path, "a")["slot"]["state"], "ready");
    }

    #[test]
    fn release_slot_clears_and_transitions_in_one_write() {
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);
        bind_slot(&path, "a", "feat/login", "main", "/wt/login").unwrap();

        release_slot(&path, "a", STATUS_FAILED).unwrap();

        let t = slot_of(&path, "a");
        // The pair that used to be two writes: never slotless-but-running.
        assert!(t.get("slot").is_none());
        assert_eq!(t["status"], STATUS_FAILED);
        assert!(t["completed_at"].is_u64());
        // The binding itself survives: it still names the tree that needs cleaning.
        assert_eq!(t["branch"], "feat/login");
    }

    #[test]
    fn attempt_survives_a_failed_run_and_bumps_on_the_next_bind() {
        // The whole fence depends on this: the counter lives on the TASK, not the
        // slot, so releasing a failed attempt must not reset it. If it reset, attempt
        // 2 would reuse attempt 1's number and a stale worker could not be told apart.
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);

        // Attempt 1 runs and fails.
        assert_eq!(
            bind_slot(&path, "a", "feat/login", "main", "/wt/login").unwrap(),
            BindOutcome::Bound { attempt: 1 }
        );
        release_slot(&path, "a", STATUS_FAILED).unwrap();
        let after_failure = slot_of(&path, "a");
        assert!(after_failure.get("slot").is_none(), "slot released");
        assert_eq!(after_failure["attempt"], 1, "counter outlives the slot");

        // Attempt 2, reusing the same tree, is a distinct attempt.
        assert_eq!(
            bind_slot(&path, "a", "feat/login", "main", "/wt/login").unwrap(),
            BindOutcome::Bound { attempt: 2 }
        );
        let t = slot_of(&path, "a");
        assert_eq!(t["attempt"], 2);
        assert_eq!(t["slot"]["attempt"], 2);
        assert_eq!(t["slot"]["state"], "starting");
    }

    #[test]
    fn release_slot_transitions_a_task_that_holds_no_slot() {
        let (_dir, path) = graph_with(&[task("a", STATUS_IN_PROGRESS, &[])]);

        release_slot(&path, "a", STATUS_TODO).unwrap();

        assert_eq!(slot_of(&path, "a")["status"], STATUS_TODO);
    }

    #[test]
    fn lock_is_reclaimed_from_a_dead_holder() {
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);
        let lock = path.with_extension(lock_extension(&path));
        // Pid 0 is never a live process for `kill(0)`, so this stands in for a
        // holder that was SIGKILLed without releasing.
        fs::write(&lock, format!("0 {}", now_secs())).unwrap();

        update_status(&path, "a", STATUS_IN_PROGRESS).unwrap();

        assert_eq!(slot_of(&path, "a")["status"], STATUS_IN_PROGRESS);
        assert!(!lock.exists(), "lock released after the write");
    }

    #[test]
    fn lock_is_reclaimed_once_it_outlives_the_staleness_bound() {
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);
        let lock = path.with_extension(lock_extension(&path));
        // Our own pid, so liveness cannot justify the steal — only age can.
        fs::write(
            &lock,
            format!("{} {}", std::process::id(), now_secs().saturating_sub(120)),
        )
        .unwrap();

        update_status(&path, "a", STATUS_IN_PROGRESS).unwrap();

        assert_eq!(slot_of(&path, "a")["status"], STATUS_IN_PROGRESS);
    }

    #[test]
    fn lock_held_by_a_live_recent_holder_is_not_reclaimed() {
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);
        let lock = path.with_extension(lock_extension(&path));
        fs::write(&lock, format!("{} {}", std::process::id(), now_secs())).unwrap();

        let err = update_status(&path, "a", STATUS_IN_PROGRESS).unwrap_err();

        assert!(err.to_string().contains("acquire lock"), "{err}");
        assert_eq!(slot_of(&path, "a")["status"], STATUS_TODO);
        let _ = fs::remove_file(&lock);
    }

    #[test]
    fn rejected_signals_are_capped_oldest_first() {
        let (_dir, path) = graph_with(&[task("a", STATUS_TODO, &[])]);

        for attempt in 1..=(MAX_REJECTED_SIGNALS as u32 + 2) {
            record_rejected_signal(&path, "a", Some(attempt), RejectionCode::StaleAttempt).unwrap();
        }

        let log = slot_of(&path, "a")["rejected_signals"].clone();
        let log = log.as_array().unwrap();
        assert_eq!(log.len(), MAX_REJECTED_SIGNALS);
        // Oldest dropped, order preserved.
        assert_eq!(log[0]["attempt"], 3);
        assert_eq!(log[MAX_REJECTED_SIGNALS - 1]["attempt"], 12);
        assert_eq!(log[0]["code"], "stale_attempt");
    }

    #[test]
    fn frontier_excludes_setup_tasks() {
        // Setup tasks are handled by preflight, never opened as normal slots,
        // even when unblocked.
        let tasks = vec![
            setup_task("provision", STATUS_TODO, "test_env", &[]),
            task("feature", STATUS_TODO, &[]),
        ];
        let ids: Vec<&str> = frontier(&tasks).iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["feature"]);
    }

    #[test]
    fn setup_capabilities_in_first_appearance_order() {
        let tasks = vec![
            setup_task("a", STATUS_TODO, "test_env", &[]),
            task("plain", STATUS_TODO, &[]),
            setup_task("b", STATUS_TODO, "db", &[]),
            setup_task("c", STATUS_TODO, "test_env", &[]),
        ];
        assert_eq!(setup_capabilities(&tasks), vec!["test_env", "db"]);
    }

    #[test]
    fn setup_frontier_respects_deps_within_capability() {
        let tasks = vec![
            setup_task("base", STATUS_DONE, "test_env", &[]),
            setup_task("step2", STATUS_TODO, "test_env", &["base"]),
            setup_task("blocked", STATUS_TODO, "test_env", &["step2"]),
            setup_task("other_cap", STATUS_TODO, "db", &[]),
        ];
        let ids: Vec<&str> = setup_frontier(&tasks, "test_env")
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(ids, vec!["step2"]);
    }

    #[test]
    fn capability_tasks_done_requires_all_done() {
        let tasks = vec![
            setup_task("s1", STATUS_DONE, "test_env", &[]),
            setup_task("s2", STATUS_IN_PROGRESS, "test_env", &[]),
        ];
        assert!(!capability_tasks_done(&tasks, "test_env"));

        let all_done = vec![
            setup_task("s1", STATUS_DONE, "test_env", &[]),
            setup_task("s2", STATUS_DONE, "test_env", &[]),
        ];
        assert!(capability_tasks_done(&all_done, "test_env"));

        // No setup tasks for the capability -> not provisioned by the graph.
        assert!(!capability_tasks_done(&all_done, "missing"));
    }

    fn child(id: &str, parent: &str, status: &str) -> GraphTask {
        GraphTask {
            parent: Some(parent.to_string()),
            ..task(id, status, &[])
        }
    }

    #[test]
    fn containment_helpers_derive_container_and_join_readiness() {
        let tasks = vec![
            task("epic", STATUS_IN_PROGRESS, &[]),
            child("a", "epic", STATUS_DONE),
            child("b", "epic", STATUS_IN_PROGRESS),
            task("solo", STATUS_TODO, &[]),
        ];
        assert!(is_container(&tasks, "epic"));
        assert!(!is_container(&tasks, "solo"));
        let kids: Vec<&str> = children_of(&tasks, "epic")
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        assert_eq!(kids, vec!["a", "b"]);
        // b still in progress → not all done.
        assert!(!all_children_done(&tasks, "epic"));
        // A childless task is never "all children done" (nothing to join).
        assert!(!all_children_done(&tasks, "solo"));
        assert!(!any_child_failed(&tasks, "epic"));
    }

    #[test]
    fn all_children_done_true_when_every_child_done() {
        let tasks = vec![
            task("epic", STATUS_BLOCKED, &[]),
            child("a", "epic", STATUS_DONE),
            child("b", "epic", STATUS_DONE),
        ];
        assert!(all_children_done(&tasks, "epic"));
    }

    #[test]
    fn reconcile_orders_consumer_after_producer() {
        let mut kids = vec![
            GraphTask {
                read_set: vec!["AuthToken".into()],
                ..task("consumer", STATUS_TODO, &[])
            },
            GraphTask {
                provides: vec!["AuthToken".into()],
                ..task("producer", STATUS_TODO, &[])
            },
        ];
        let notes = reconcile_conflicts(&mut kids);
        let consumer = kids.iter().find(|t| t.id == "consumer").unwrap();
        assert!(consumer.depends_on.contains(&"producer".to_string()));
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn reconcile_serializes_write_set_overlap() {
        // Two children both write `routes.rs`, no ordering → serialize the later.
        let mut kids = vec![
            GraphTask {
                write_set: vec!["src/routes.rs".into()],
                ..task("first", STATUS_TODO, &[])
            },
            GraphTask {
                write_set: vec!["src/routes.rs".into(), "src/x.rs".into()],
                ..task("second", STATUS_TODO, &[])
            },
        ];
        let notes = reconcile_conflicts(&mut kids);
        let second = kids.iter().find(|t| t.id == "second").unwrap();
        assert!(second.depends_on.contains(&"first".to_string()));
        assert_eq!(notes.len(), 1);
        // Idempotent: a second pass adds nothing.
        let notes2 = reconcile_conflicts(&mut kids);
        assert!(notes2.is_empty());
    }

    #[test]
    fn reconcile_leaves_disjoint_writes_untouched() {
        let mut kids = vec![
            GraphTask {
                write_set: vec!["a.rs".into()],
                ..task("x", STATUS_TODO, &[])
            },
            GraphTask {
                write_set: vec!["b.rs".into()],
                ..task("y", STATUS_TODO, &[])
            },
        ];
        let notes = reconcile_conflicts(&mut kids);
        assert!(notes.is_empty());
        assert!(kids.iter().all(|t| t.depends_on.is_empty()));
    }

    #[test]
    fn update_status_preserves_list_and_unmodelled_fields() {
        // `acceptance_criteria` authored as a list and an unmodelled
        // `harness_layers` field must round-trip byte-identical: update_status
        // mutates through serde_json::Value, never through the typed GraphTask.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index.json");
        let raw = serde_json::json!([{
            "id": "a",
            "title": "Title a",
            "status": STATUS_TODO,
            "acceptance_criteria": ["criterion one", "criterion two"],
            "harness_layers": ["H13"]
        }]);
        fs::write(&path, serde_json::to_string_pretty(&raw).unwrap()).unwrap();

        update_status(&path, "a", STATUS_DONE).unwrap();

        let out: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let task = &out.as_array().unwrap()[0];
        assert_eq!(task["status"], Value::String(STATUS_DONE.to_string()));
        assert_eq!(
            task["acceptance_criteria"],
            serde_json::json!(["criterion one", "criterion two"])
        );
        assert_eq!(task["harness_layers"], serde_json::json!(["H13"]));
    }

    #[test]
    fn stats_counts_by_status() {
        let tasks = vec![
            task("a", STATUS_DONE, &[]),
            task("b", STATUS_IN_PROGRESS, &[]),
            task("c", STATUS_TODO, &[]),
            task("d", STATUS_FAILED, &[]),
        ];
        assert_eq!(stats(&tasks), (4, 1, 1, 1));
        assert_eq!(todo_count(&tasks), 1);
    }
}
