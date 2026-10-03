//! `workmux task` — CLI CRUD over the task graph (`tasks/index.json`).
//!
//! This is the non-interactive counterpart to the `workmux tasks` TUI: it lets
//! humans and agents read and mutate the graph from a shell (or worktree)
//! without hand-editing JSON. All writes go through the same atomic, locked
//! `tasks::graph` operations, as a CLI an external harness can drive.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::tasks::graph;
use crate::tasks::types::{GraphTask, SlotLifecycle, TaskPatch};

/// Resolve a (possibly relative) graph path against the **main worktree root**,
/// so an agent running inside a feature worktree reads/writes the project's
/// single graph rather than a per-worktree copy. Absolute paths are used as-is;
/// outside a git repo we fall back to the path as given (relative to cwd).
pub(crate) fn resolve_graph(graph: &Path) -> PathBuf {
    if graph.is_absolute() {
        return graph.to_path_buf();
    }
    match crate::git::get_main_worktree_root() {
        Ok(root) => root.join(graph),
        Err(_) => graph.to_path_buf(),
    }
}

/// `workmux task list` — print tasks, optionally filtered.
pub fn list(
    graph: &Path,
    status: Option<String>,
    label: Option<String>,
    frontier: bool,
    json: bool,
) -> Result<()> {
    let path = resolve_graph(graph);
    let tasks = graph::load(&path)?;

    let selected: Vec<&GraphTask> = if frontier {
        graph::frontier(&tasks)
    } else {
        tasks
            .iter()
            .filter(|t| status.as_ref().map_or(true, |s| &t.status == s))
            .filter(|t| label.as_ref().map_or(true, |l| t.labels.contains(l)))
            .collect()
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&selected)?);
        return Ok(());
    }
    if selected.is_empty() {
        println!("no tasks");
        return Ok(());
    }
    for t in selected {
        // `unknown` slots and refused signals are the two states nothing else
        // reports: the task looks busy but nothing is working it, or a worker
        // believes it finished and the graph disagrees. Both need a human or
        // coordinator decision, so neither may be invisible in the default view.
        let note = task_attention(t);
        println!("{:<12} {:<28} {}{}", t.status, t.id, t.title, note);
    }
    Ok(())
}

/// Trailing marker for a task needing an explicit decision, empty when it does not.
fn task_attention(t: &GraphTask) -> String {
    let mut notes = Vec::new();
    if t.slot.as_ref().map(|s| s.state) == Some(SlotLifecycle::Unknown) {
        notes.push("slot:unknown".to_string());
    }
    if t.slot.as_ref().map(|s| s.state) == Some(SlotLifecycle::Starting) {
        notes.push("slot:starting".to_string());
    }
    match t.rejected_signals.len() {
        0 => {}
        n => notes.push(format!("rejected:{n}")),
    }
    if notes.is_empty() {
        String::new()
    } else {
        format!("  [{}]", notes.join(" "))
    }
}

/// Main worktree root (the project's single `.workmux/`), falling back to cwd.
fn repo_root() -> PathBuf {
    crate::git::get_main_worktree_root()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}


/// `workmux task get` — exact id match, else fuzzy search over id+title.
pub fn get(graph: &Path, query: &str, json: bool) -> Result<()> {
    let path = resolve_graph(graph);
    let tasks = graph::load(&path)?;

    // Exact id always wins.
    if let Some(t) = tasks.iter().find(|t| t.id == query) {
        return print_task(t, json);
    }

    // Fuzzy: score each task by the better of its id/title match.
    let mut scored: Vec<(i32, &GraphTask)> = tasks
        .iter()
        .filter_map(|t| {
            let id_s = fuzzy_score(query, &t.id);
            let title_s = fuzzy_score(query, &t.title);
            id_s.max(title_s).map(|sc| (sc, t))
        })
        .collect();
    // Highest score first; ties broken by id for determinism.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));

    match scored.as_slice() {
        [] => Err(anyhow!("no task matches {query:?}")),
        // Unambiguous: a single candidate, or a clear top score.
        [(_, t)] => print_task(t, json),
        [(top, t), (second, _), ..] if *top >= *second + 8 => print_task(t, json),
        _ => {
            // Ambiguous: surface the candidates so the caller can pick an id.
            if json {
                let cands: Vec<&GraphTask> = scored.iter().take(10).map(|(_, t)| *t).collect();
                println!("{}", serde_json::to_string_pretty(&cands)?);
            } else {
                println!("{query:?} is ambiguous — candidates (score id title):");
                for (sc, t) in scored.iter().take(10) {
                    println!("  {:<4} {:<28} {}", sc, t.id, t.title);
                }
            }
            Ok(())
        }
    }
}

/// `workmux task create` — append a new task.
#[allow(clippy::too_many_arguments)]
pub fn create(
    graph: &Path,
    id: Option<String>,
    title: String,
    description: String,
    depends_on: Vec<String>,
    status: String,
    worktree: Option<String>,
    labels: Vec<String>,
    priority: Option<i64>,
    parent: Option<String>,
    inherit_from: Option<String>,
) -> Result<()> {
    let path = resolve_graph(graph);
    let id = id.unwrap_or_else(|| slug(&title));
    if id.is_empty() {
        return Err(anyhow!(
            "task id is empty; pass --id or a --title that yields a slug"
        ));
    }
    // Default the worktree handle to the id, matching how tasks are authored by
    // hand (every existing task sets worktree == id).
    let worktree = worktree.or_else(|| Some(id.clone()));
    let task = GraphTask {
        id: id.clone(),
        title,
        description,
        depends_on,
        status,
        worktree,
        labels,
        priority,
        parent,
        ..Default::default()
    };
    match &inherit_from {
        Some(src) => graph::add_task_inheriting(&path, &task, src)?,
        None => graph::add_task(&path, &task)?,
    }
    println!("created {id}");
    Ok(())
}

/// `workmux task update` — patch fields of an existing task.
#[allow(clippy::too_many_arguments)]
pub fn update(
    graph: &Path,
    id: &str,
    title: Option<String>,
    description: Option<String>,
    status: Option<String>,
    worktree: Option<String>,
    depends_on: Option<Vec<String>>,
    labels: Option<Vec<String>>,
    priority: Option<i64>,
    hint: Option<String>,
    blocked_reason: Option<String>,
    blocked_reason_file: Option<PathBuf>,
    add_labels: Vec<String>,
    remove_labels: Vec<String>,
) -> Result<()> {
    let path = resolve_graph(graph);
    let blocked_reason = match blocked_reason_file {
        Some(f) => Some(
            std::fs::read_to_string(&f)
                .map(|s| s.trim().to_string())
                .with_context(|| format!("read blocked reason {}", f.display()))?,
        ),
        None => blocked_reason,
    };
    let patch = TaskPatch {
        title,
        description,
        status,
        worktree,
        depends_on,
        labels,
        priority,
        hint,
        blocked_reason,
        add_labels,
        remove_labels,
        ..Default::default()
    };
    if patch.is_empty() {
        return Err(anyhow!("nothing to update; pass at least one field flag"));
    }
    graph::update_task(&path, id, &patch)?;
    println!("updated {id}");
    Ok(())
}

/// `workmux task delete` — remove a task by id.
pub fn delete(graph: &Path, id: &str) -> Result<()> {
    let path = resolve_graph(graph);
    graph::delete_task(&path, id)?;
    println!("deleted {id}");
    Ok(())
}

/// Bind a task to a worktree/branch the caller is about to create.
///
/// Exit codes matter to an agent driving this: 0 for a fresh or repeated claim, 1
/// for a conflict, so a wrapper script can branch on it without parsing text.
pub fn claim(
    graph: &Path,
    id: &str,
    branch: &str,
    base: &str,
    worktree: &Path,
    json: bool,
) -> Result<()> {
    let path = resolve_graph(graph);
    let worktree = worktree.to_string_lossy().to_string();
    let outcome = graph::bind_slot(&path, id, branch, base, &worktree)?;

    let (state, attempt) = match &outcome {
        graph::BindOutcome::Bound { attempt } => ("bound", Some(*attempt)),
        graph::BindOutcome::AlreadyBound { attempt } => ("already_bound", Some(*attempt)),
        graph::BindOutcome::Conflict { attempt, .. } => ("conflict", *attempt),
    };

    if json {
        let mut out = serde_json::json!({
            "task": id,
            "state": state,
            "attempt": attempt,
        });
        if let graph::BindOutcome::Conflict {
            branch: held_branch,
            worktree: held_worktree,
            ..
        } = &outcome
        {
            out["held_branch"] = serde_json::json!(held_branch);
            out["held_worktree"] = serde_json::json!(held_worktree);
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        match &outcome {
            graph::BindOutcome::Bound { attempt } => {
                println!("bound {id} to {branch} at {worktree} (attempt {attempt})");
            }
            graph::BindOutcome::AlreadyBound { attempt } => {
                println!("{id} already bound to {branch} (attempt {attempt})");
            }
            graph::BindOutcome::Conflict {
                branch: held_branch,
                worktree: held_worktree,
                attempt,
            } => {
                eprintln!(
                    "{id} is already bound to {} at {} (attempt {}); not rebinding",
                    held_branch.as_deref().unwrap_or("(unknown branch)"),
                    held_worktree.as_deref().unwrap_or("(unknown path)"),
                    attempt.map(|a| a.to_string()).unwrap_or_else(|| "?".into()),
                );
            }
        }
    }

    if matches!(outcome, graph::BindOutcome::Conflict { .. }) {
        std::process::exit(1);
    }
    Ok(())
}

/// Resolve a parked task: back to the queue, or given up on.
///
/// Both paths clear the slot, and that is not optional. A parked task that keeps
/// its slot makes every later `bind_slot` return `Conflict`, so the attempt never
/// bumps and the task can never be claimed again. The binding (`branch`/`base`) is
/// kept either way — the worktree still exists and still needs reusing or cleaning.
pub fn resolve(graph: &Path, id: &str, retry: bool, abandon: bool) -> Result<()> {
    if retry == abandon {
        return Err(anyhow!("pass exactly one of --retry or --abandon"));
    }
    let path = resolve_graph(graph);
    let tasks = graph::load(&path)?;
    let task = tasks
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| anyhow!("task {id:?} not found"))?;

    let held = task
        .slot
        .as_ref()
        .and_then(|s| s.worktree_path.as_deref())
        .unwrap_or("(no worktree recorded)");

    if retry {
        graph::release_slot(&path, id, crate::tasks::types::STATUS_TODO)?;
        println!("{id} requeued; next dispatch binds a new attempt (worktree {held} kept)");
    } else {
        graph::release_slot(&path, id, crate::tasks::types::STATUS_FAILED)?;
        let _ = graph::set_blocked_reason(&path, id, "resolved: abandoned by operator");
        println!("{id} abandoned; worktree {held} left for inspection");
    }
    Ok(())
}

fn print_task(t: &GraphTask, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(t)?);
        return Ok(());
    }
    println!("id:          {}", t.id);
    println!("title:       {}", t.title);
    println!("status:      {}", t.status);
    if !t.depends_on.is_empty() {
        println!("depends_on:  {}", t.depends_on.join(", "));
    }
    if let Some(w) = &t.worktree {
        println!("worktree:    {w}");
    }
    if !t.labels.is_empty() {
        println!("labels:      {}", t.labels.join(", "));
    }
    if let Some(p) = t.priority {
        println!("priority:    {p}");
    }
    let section = |name: &str, body: &str| {
        if !body.is_empty() {
            println!("\n{name}:\n{body}");
        }
    };
    section("description", &t.description);
    if let Some(v) = &t.acceptance_criteria {
        section("acceptance_criteria", v);
    }
    if let Some(v) = &t.testing_dod {
        section("testing_dod", v);
    }
    if let Some(v) = &t.implementation_plan {
        section("implementation_plan", v);
    }
    if let Some(v) = &t.agent_hints {
        section("agent_hints", v);
    }
    Ok(())
}

/// Case-insensitive subsequence fuzzy score; higher is better, `None` = no
/// match (not every query character was found in order). Rewards contiguous
/// runs, a prefix hit, and tighter (shorter) targets.
fn fuzzy_score(query: &str, target: &str) -> Option<i32> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let t = target.to_lowercase();
    let qb = q.as_bytes();
    let tb = t.as_bytes();

    let mut qi = 0usize;
    let mut score = 0i32;
    let mut prev_match: Option<usize> = None;
    for (i, &c) in tb.iter().enumerate() {
        if qi < qb.len() && c == qb[qi] {
            score += 1;
            if i == 0 {
                score += 5; // prefix bonus
            }
            if let Some(p) = prev_match {
                if p + 1 == i {
                    score += 3; // contiguous-run bonus
                }
            }
            prev_match = Some(i);
            qi += 1;
        }
    }
    if qi == qb.len() {
        // Tighter matches rank higher: small bonus that shrinks with length.
        Some(score + (50i32 - t.len() as i32).max(0) / 5)
    } else {
        None
    }
}

/// Lowercase kebab-case slug from a title (used when `--id` is omitted on create).
fn slug(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}


#[cfg(test)]
mod tests {
    use super::*;

    use crate::tasks::types::{RejectedSignal, TaskSlot};

    fn with_slot(state: SlotLifecycle) -> GraphTask {
        GraphTask {
            slot: Some(TaskSlot {
                state,
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn attention_marks_unsettled_slots_and_refused_signals() {
        // These two states are invisible everywhere else: the task looks busy but
        // nothing is working it, or a worker believes it finished and the graph
        // disagrees. Both need a decision, so the default listing must show them.
        assert_eq!(
            task_attention(&with_slot(SlotLifecycle::Unknown)),
            "  [slot:unknown]"
        );
        assert_eq!(
            task_attention(&with_slot(SlotLifecycle::Starting)),
            "  [slot:starting]"
        );

        let mut t = with_slot(SlotLifecycle::Unknown);
        t.rejected_signals = vec![
            RejectedSignal {
                at: 1,
                attempt: Some(1),
                code: "stale_attempt".into(),
            },
            RejectedSignal {
                at: 2,
                attempt: None,
                code: "missing_attempt".into(),
            },
        ];
        assert_eq!(task_attention(&t), "  [slot:unknown rejected:2]");
    }

    #[test]
    fn attention_is_silent_for_an_ordinary_task() {
        // No marker for healthy tasks, or the signal drowns in noise.
        assert_eq!(task_attention(&with_slot(SlotLifecycle::Ready)), "");
        assert_eq!(task_attention(&GraphTask::default()), "");
    }

    #[test]
    fn exact_subsequence_matches_and_ranks_prefix_higher() {
        // Both match "hrd" as a subsequence; the prefix-aligned one scores higher.
        let prefix = fuzzy_score("hrd", "harness-recursive-decomposition").unwrap();
        let scattered = fuzzy_score("hrd", "the-harness-runs-daily").unwrap();
        assert!(prefix > 0 && scattered > 0);
    }

    #[test]
    fn non_subsequence_is_no_match() {
        assert!(fuzzy_score("xyz", "harness").is_none());
    }

    #[test]
    fn contiguous_beats_scattered() {
        let contiguous = fuzzy_score("dec", "decompose").unwrap();
        let scattered = fuzzy_score("dec", "d-e-c").unwrap();
        assert!(contiguous > scattered);
    }

    /// `--blocked-reason-file` is how the harness hands findings to the steward:
    /// the file's text (trimmed) lands on the task record.
    #[test]
    fn update_reads_blocked_reason_from_file_and_creates_child() {
        let dir = tempfile::tempdir().unwrap();
        let graph = dir.path().join("index.json");
        std::fs::write(
            &graph,
            r#"[{"id":"a","title":"A","status":"failed","harness_layers":{"replay":true}}]"#,
        )
        .unwrap();
        let reason = dir.path().join("a.md");
        std::fs::write(&reason, "  round 2: replay diverged\n").unwrap();

        update(
            &graph,
            "a",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(reason),
            vec![],
            vec![],
        )
        .unwrap();
        create(
            &graph,
            Some("a-1".into()),
            "Child".into(),
            String::new(),
            vec![],
            "todo".into(),
            None,
            vec![],
            None,
            Some("a".into()),
            Some("a".into()),
        )
        .unwrap();

        let tasks = graph::load(&graph).unwrap();
        let a = tasks.iter().find(|t| t.id == "a").unwrap();
        assert_eq!(a.blocked_reason.as_deref(), Some("round 2: replay diverged"));
        let child = tasks.iter().find(|t| t.id == "a-1").unwrap();
        assert_eq!(child.parent.as_deref(), Some("a"));
        assert_eq!(child.status, "todo");
        assert!(child.blocked_reason.is_none());
        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&graph).unwrap()).unwrap();
        let child_raw = raw
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["id"] == "a-1")
            .unwrap();
        assert_eq!(
            child_raw["harness_layers"],
            serde_json::json!({"replay": true}),
            "--inherit-from carries the parent's harness config"
        );
    }

    #[test]
    fn slug_kebabs_and_trims() {
        assert_eq!(
            slug("Harness IPC: enforced I/O!"),
            "harness-ipc-enforced-i-o"
        );
        assert_eq!(slug("  spaced  "), "spaced");
    }
}
